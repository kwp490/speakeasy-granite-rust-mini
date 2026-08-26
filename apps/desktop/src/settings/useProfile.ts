import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { readWithRetry } from "./readWithRetry";
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
  /**
   * The profile could not be read, so every control fed from it is showing its
   * own default rather than the user's setting.
   *
   * Reported rather than merely tolerated. A null profile renders unchecked boxes
   * and a delivery preference nobody chose, across three pages — settings that
   * are not the user's, presented as though they were.
   */
  unavailable: boolean;
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

export function useProfile(): ProfileController {
  const [profile, setProfile] = useState<ProfileStatus | null>(null);
  const [unavailable, setUnavailable] = useState(false);

  /**
   * Loads the profile through the shared retry.
   *
   * Both windows' webviews load while Tauri's `setup` is still running, so an
   * immediate `profile_status` can arrive before `ProfileCoordinator` is managed
   * and be refused outright — observed on a cold start as "state not managed for
   * field `state` on command `profile_status`". Without a retry the page keeps a
   * null profile forever and renders defaults: unchecked boxes and a delivery
   * preference nobody chose. Nothing is written from that state, but showing a
   * user settings that are not theirs is its own failure — so it is now also
   * *said*, through `unavailable`.
   *
   * This is `readWithRetry` rather than the hand-rolled interval it used to be.
   * The two carried the same 20 x 250 ms by hand, in two files, and
   * `readWithRetry`'s own comment named the risk: one page recovering from a
   * startup the other reported as broken. Two implementations of one race is a
   * second source of truth, which is the shape of defect this whole sweep exists
   * to remove.
   */
  const reload = useCallback(() => {
    void readWithRetry<ProfileStatus>("profile_status").then(
      (status) => {
        setProfile(status);
        setUnavailable(false);
      },
      () => {
        setUnavailable(true);
      },
    );
  }, []);

  useEffect(reload, [reload]);

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
    unavailable,
    reload,
    setStartup,
    setRecordingFeedback,
    setDiskLogging,
    setDelivery,
    setHistory,
    replace: setProfile,
  };
}
