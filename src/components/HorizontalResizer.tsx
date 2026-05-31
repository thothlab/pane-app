import { type Component } from "solid-js";

export const HorizontalResizer: Component<{
  onResize: (deltaY: number) => void;
  onReset?: () => void;
  title?: string;
}> = (props) => {
  const start = (e: MouseEvent) => {
    e.preventDefault();
    let lastY = e.clientY;
    const move = (ev: MouseEvent) => {
      const delta = ev.clientY - lastY;
      lastY = ev.clientY;
      if (delta !== 0) props.onResize(delta);
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
    document.body.style.cursor = "row-resize";
    document.body.style.userSelect = "none";
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };
  return (
    <div
      class="group relative h-1.5 cursor-row-resize self-stretch z-10"
      onMouseDown={start}
      onDblClick={() => props.onReset?.()}
      title={props.title ?? "Drag to resize · double-click to reset"}
    >
      <div class="absolute inset-x-0 top-1/2 h-px bg-border group-hover:bg-accent group-active:bg-accent" />
    </div>
  );
};
