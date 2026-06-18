// Clipboard writes via the Tauri plugin instead of `navigator.clipboard`.
// The webview API rejects with NotAllowedError when the transient
// user-activation has been consumed by `await` chains before the write —
// reproduced intermittently on testers' machines when the captures
// "Copy" menu item runs `buildHttpDump` (IPC + body decode) before
// writing. The plugin call goes through Rust IPC and is not subject to
// the gesture/focus rules.
import {
  writeText as pluginWriteText,
  readText as pluginReadText,
} from "@tauri-apps/plugin-clipboard-manager";

export async function writeClipboard(text: string): Promise<void> {
  await pluginWriteText(text);
}

export async function readClipboard(): Promise<string> {
  const v = await pluginReadText();
  return v ?? "";
}
