// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";

import { MessageBubble } from "@/components/chat/message-bubble";
import type { LiveChatMessage } from "@/hooks/use-chat";

let container: HTMLDivElement | null = null;
let root: Root | null = null;

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  container?.remove();
  container = null;
  root = null;
});

function render(message: LiveChatMessage) {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);

  act(() => {
    root?.render(<MessageBubble message={message} density="compact" />);
  });

  return container;
}

describe("MessageBubble", () => {
  it("preserves user-authored newlines in the Quick Ask compact bubble", () => {
    const content = "第一行\n第二行\n第三行";
    const node = render({
      id: "msg-1",
      role: "user",
      content,
      toolEvents: [],
      segments: [],
      citedFrameIds: [],
      isStreaming: false,
    }).querySelector(".whitespace-pre-wrap");

    expect(node).not.toBeNull();
    expect(node?.textContent).toBe(content);
    expect(node?.className).toContain("break-words");
  });
});
