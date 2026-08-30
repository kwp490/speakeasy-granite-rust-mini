import { defineConfig, mergeConfig } from "vitest/config";

import viteConfig from "./vite.config";

/**
 * The component-test harness: React rendered into a real DOM, driven the way a
 * user drives it.
 *
 * A second runner beside `node --test`, and deliberately not a replacement for
 * it. The `.test.mjs` suite imports plain `.ts` modules under Node's own type
 * stripping and asserts against source text; it is fast, it needs no browser,
 * and the reducer and level-shaping proofs belong there. What it cannot do is
 * press a button. Every rule this app has about honesty — that a control never
 * claims something it did not do, that a refusal is reported rather than
 * swallowed, that a destructive confirmation does not clear itself on failure —
 * is a rule about what a *rendered* control does when a command rejects, and
 * none of it was observable before this existed. The proof of that: four of the
 * `useMutation` conversions this harness was added alongside had shipped with
 * no rejection handler at all, and a full green suite.
 *
 * `mergeConfig` over the app's own config rather than a second copy of it. The
 * product version is read out of `Cargo.toml` there and injected as
 * `__SPEAKEASY_PRODUCT_VERSION__`, which `catalog.ts` reads at module scope --
 * so a standalone config would either duplicate that read or fail to define the
 * global, and a duplicate is a second thing that can disagree.
 */
export default mergeConfig(
  viteConfig,
  defineConfig({
    test: {
      // A real DOM. `happy-dom` is faster and is not what the app runs in;
      // `jsdom` is pinned at 29 rather than 30 because 30 requires a Node newer
      // than this workspace's exact `=22.16.0` engine pin.
      environment: "jsdom",
      // Only the component tests. `tests/*.test.mjs` stays with `node --test`,
      // which is what `npm run test:unit` runs.
      include: ["tests/components/**/*.test.tsx"],
      // Explicit imports, no ambient `describe`/`it`. The `.mjs` suite imports
      // `test` from `node:test`; a global here would make the two suites read
      // differently for no reason.
      globals: false,
      // Every stub is undone between tests. A `vi.mock` factory that leaked its
      // last return value into the next test is a test asserting the previous
      // test's setup.
      restoreMocks: true,
      mockReset: true,
      coverage: {
        provider: "v8",
        // JSON only, and written where `Test-CoverageFloors.ps1` reads it. No
        // HTML report: nothing in this repository opens one, and an artefact
        // nobody reads is an artefact that goes stale without saying so.
        reporter: ["json-summary"],
        reportsDirectory: "../../target/coverage/frontend",
        // Only what the floors are about. Including every file would make the
        // total a number that moves whenever a component is added, which is the
        // thing that turns a ratchet into a nuisance and then into a disabled
        // check.
        include: ["src/settings/**/*.{ts,tsx}"],
        // Nothing is enforced here. The floors live in
        // `dependency-policy/coverage-floors.json` next to the Rust ones, so
        // there is one file to read when asking what this project guarantees.
        thresholds: undefined,
      },
    },
  }),
);
