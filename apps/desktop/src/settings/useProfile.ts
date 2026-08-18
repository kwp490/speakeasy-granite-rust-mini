import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type { ProfileStatus, SafeDeliveryPreference } from "./types";

/**
 * The profile the General, Output & Privacy and Advanced pages all read.
 *
 * One loader shared by three pages, because `ProfileView` is one document in the
 * backend and three independent copies of it would drift the moment one page
 * saved. Every mutator returns the fresh `ProfileView` the command produced, so
 * the local copy is replaced by the backend's rather than patched optimistically.
 */
export type ProfileController = {
  profile: ProfileStatus | null;
  reload: () => void;
  setStartup: (enabled: boolean) => Promise<void>;
  setRecordingFeedback: (enabled: boolean) => Promise<void>;
  setDiskLogging: (enabled: boolean) => Promise<void>;
  setDelivery: (preference: SafeDeliveryPreference) => Promise<void>;
  setHistory: (options: {
    enabled: boolean;
    retentionDays: number;
    disclosureAccepted: boolean;
  }) => Promise<void>;
  replace: (next: ProfileStatus) => void;
};

/** Attempts before giving up on the profile. 20 x 250 ms covers a slow cold start. */
const PROFILE_LOAD_ATTEMPTS = 20;

export function useProfile(): ProfileController {
  const [profile, setProfile] = useState<ProfileStatus | null>(null);
  const [loadFailed, setLoadFailed] = useState(false);

  const reload = useCallback(() => {
    void invoke<ProfileStatus>("profile_status")
      .then((status) => {
        setProfile(status);
        setLoadFailed(false);
      })
      .catch(() => {
        setLoadFailed(true);
      });
  }, []);

  useEffect(reload, [reload]);

  /**
   * Retries a profile load that lost its race with app startup.
   *
   * Both windows' webviews load while Tauri's `setup` is still running, so an
   * immediate `profile_status` can arrive before `ProfileCoordinator` is managed
   * and be refused outright — observed on a cold start as "state not managed for
   * field `state` on command `profile_status`". Without a retry the page keeps a
   * null profile forever and renders defaults: unchecked boxes and no setup
   * stepper, for a profile that has neither of those things. Nothing is written
   * from that state, but showing a user settings that are not theirs is its own
   * failure.
   */
  useEffect(() => {
    if (!loadFailed) return;
    let attempts = 0;
    const timer = window.setInterval(() => {
      attempts += 1;
      if (attempts >= PROFILE_LOAD_ATTEMPTS) {
        window.clearInterval(timer);
        return;
      }
      reload();
    }, 250);
    return () => {
      window.clearInterval(timer);
    };
  }, [loadFailed, reload]);

  const setStartup = useCallback(async (enabled: boolean) => {
    setProfile(await invoke<ProfileStatus>("startup_configure", { enabled }));
  }, []);

  const setRecordingFeedback = useCallback(async (enabled: boolean) => {
    setProfile(await invoke<ProfileStatus>("recording_feedback_configure", { enabled }));
  }, []);

  const setDiskLogging = useCallback(async (enabled: boolean) => {
    setProfile(await invoke<ProfileStatus>("disk_logging_configure", { enabled }));
  }, []);

  const setDelivery = useCallback(async (preference: SafeDeliveryPreference) => {
    setProfile(await invoke<ProfileStatus>("delivery_configure", { preference }));
  }, []);

  const setHistory = useCallback(
    async (options: { enabled: boolean; retentionDays: number; disclosureAccepted: boolean }) => {
      setProfile(
        await invoke<ProfileStatus>("history_configure", {
          enabled: options.enabled,
          retentionDays: options.retentionDays,
          plaintextDisclosureAccepted: options.disclosureAccepted,
        }),
      );
    },
    [],
  );

  return {
    profile,
    reload,
    setStartup,
    setRecordingFeedback,
    setDiskLogging,
    setDelivery,
    setHistory,
    replace: setProfile,
  };
}
