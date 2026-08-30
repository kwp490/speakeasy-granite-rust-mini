import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { readWithRetry } from "./readWithRetry";
import type { ProfileStatus, SafeDeliveryPreference } from "./types";
import { useMutation, type Mutation } from "./useMutation";

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
  /**
   * The last profile write, so a refused one is *said* rather than merely not
   * applied.
   *
   * Every mutator below was `setProfile(await invoke(...))` with no rejection
   * handler. Two things followed. The refusal was an unhandled promise
   * rejection; and because each control is rendered from `profile`, a refused
   * toggle simply snapped back to its stored value with nothing on screen — the
   * user sees a switch that will not move and no reason why. That is honest
   * about the *state* and silent about the *event*, which is the half of the
   * disclosure rule that is easy to miss: not claiming a success is not the
   * same as reporting a failure.
   *
   * One mutation for all five writers rather than one each, because they write
   * one document and `useMutation` refuses a second submission while one is in
   * flight — which is what stops two toggles racing to replace the same
   * `ProfileView`. `SettingsApp` renders the error once for the whole
   * workspace, beside the `unavailable` banner and for the same reason: the
   * profile feeds three pages.
   */
  write: Mutation<ProfileStatus>;
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

  const write = useMutation<ProfileStatus>();

  /**
   * Runs one profile write and adopts the result **only if it happened**.
   *
   * `run` resolves to `null` on a refusal, so the `!== null` is what keeps a
   * failed write from replacing the local copy with anything. There is nothing
   * optimistic to undo, because nothing is applied before the backend answers.
   */
  const { run } = write;
  const configure = useCallback(
    async (command: () => Promise<ProfileStatus>) => {
      const next = await run(command);
      if (next !== null) setProfile(next);
    },
    [run],
  );

  const setStartup = useCallback(
    async (enabled: boolean) => {
      await configure(() => invoke<ProfileStatus>("startup_configure", { enabled }));
    },
    [configure],
  );

  const setRecordingFeedback = useCallback(
    async (enabled: boolean) => {
      await configure(() => invoke<ProfileStatus>("recording_feedback_configure", { enabled }));
    },
    [configure],
  );

  const setDiskLogging = useCallback(
    async (enabled: boolean) => {
      await configure(() => invoke<ProfileStatus>("disk_logging_configure", { enabled }));
    },
    [configure],
  );

  const setDelivery = useCallback(
    async (preference: SafeDeliveryPreference) => {
      await configure(() => invoke<ProfileStatus>("delivery_configure", { preference }));
    },
    [configure],
  );

  const setHistory = useCallback(
    async (options: { enabled: boolean; retentionDays: number; disclosureAccepted: boolean }) => {
      await configure(() =>
        invoke<ProfileStatus>("history_configure", {
          enabled: options.enabled,
          retentionDays: options.retentionDays,
          plaintextDisclosureAccepted: options.disclosureAccepted,
        }),
      );
    },
    [configure],
  );

  return {
    profile,
    unavailable,
    write,
    reload,
    setStartup,
    setRecordingFeedback,
    setDiskLogging,
    setDelivery,
    setHistory,
    replace: setProfile,
  };
}
