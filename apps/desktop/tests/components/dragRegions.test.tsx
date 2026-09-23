import { expect, test, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import { messages } from "../../src/catalog";
import { PinnedLogApp } from "../../src/hud/PinnedLogApp";

const windowDouble = vi.hoisted(() => ({
  startDragging: vi.fn(() => Promise.resolve()),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    startDragging: windowDouble.startDragging,
    outerPosition: () => Promise.resolve({ x: 0, y: 0 }),
  }),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: () => Promise.resolve([]),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => {}),
}));

// The pinned log is undecorated, so its header row is the only way to move it.
// `useDragToMove` starts a drag only inside `[data-tauri-drag-region]`; a header
// carrying any other attribute renders fine and never moves.
test("the pinned log header drags the window and its close control does not", () => {
  render(<PinnedLogApp />);

  fireEvent.mouseDown(screen.getByRole("button", { name: messages.transcriptLogUnpin }), {
    button: 0,
  });
  expect(windowDouble.startDragging).not.toHaveBeenCalled();

  fireEvent.mouseDown(screen.getByText(messages.settingsGroups.log), { button: 0 });
  expect(windowDouble.startDragging).toHaveBeenCalledTimes(1);
});
