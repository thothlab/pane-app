/**
 * Vertical drag-handle for splitting panes side-by-side.
 *
 * Renders a 6px-wide hit zone with a 1px visible line at its centre. Cursor
 * becomes `col-resize` on hover; on drag the body cursor + `user-select:none`
 * are set globally so the cursor stays stable even when the pointer escapes
 * the handle's bounds.
 *
 * The component is layout-agnostic: it streams raw `deltaX` to the parent on
 * each mousemove. The parent decides whether to apply it to a px width, a
 * fraction, or something more exotic, and how to clamp.
 */

import { type Component } from "solid-js";

export const VerticalResizer: Component<{
  onResize: (deltaX: number) => void;
  onReset?: () => void;
  title?: string;
}> = (props) => {
  const start = (e: MouseEvent) => {
    e.preventDefault();
    let lastX = e.clientX;
    const move = (ev: MouseEvent) => {
      const delta = ev.clientX - lastX;
      lastX = ev.clientX;
      if (delta !== 0) props.onResize(delta);
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };
  return (
    <div
      class="group relative w-1.5 cursor-col-resize self-stretch z-10"
      onMouseDown={start}
      onDblClick={props.onReset}
      title={props.title ?? "Drag to resize · double-click to reset"}
    >
      <div class="absolute inset-y-0 right-1/2 w-px bg-border group-hover:bg-accent group-active:bg-accent" />
    </div>
  );
};
