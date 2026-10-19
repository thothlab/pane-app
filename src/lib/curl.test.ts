import { describe, expect, it } from "vitest";
import { curlCommand } from "@/lib/curl";

describe("curlCommand", () => {
  it("matches the backend format and escapes single quotes", () => {
    expect(
      curlCommand(
        { method: "POST", scheme: "https", server_host: "api.example.com", server_port: 443, url_path: "/v1/x?a=1" },
        [
          { name: "Accept", value: "*/*" },
          { name: "X-Q", value: "it's" },
        ],
      ),
    ).toBe(
      "curl -X POST 'https://api.example.com:443/v1/x?a=1' -H 'Accept: */*' -H 'X-Q: it'\\''s'",
    );
  });
});
