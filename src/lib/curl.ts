import type { CaptureDto, HeaderDto } from "@/ipc/types";

/**
 * Same command `export_one(…, "curl")` builds in pane-storage, minus the
 * round trip — built here so the headers can go through `redactHeaders`
 * first. Keep the two in step.
 */
export function curlCommand(
  cap: Pick<CaptureDto, "method" | "scheme" | "server_host" | "server_port" | "url_path">,
  headers: readonly HeaderDto[],
): string {
  let s = `curl -X ${cap.method} '${cap.scheme}://${cap.server_host}:${cap.server_port}${cap.url_path}'`;
  for (const h of headers) {
    s += ` -H '${h.name}: ${h.value.replace(/'/g, "'\\''")}'`;
  }
  return s;
}
