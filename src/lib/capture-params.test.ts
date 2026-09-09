import { describe, expect, it } from "vitest";
import { paramsFromQuery, splitPathQuery } from "@/lib/capture-params";

describe("paramsFromQuery", () => {
  it("carries every pair of a captured query", () => {
    // The reported case: the rule used to land with no params at all.
    expect(paramsFromQuery("qrStatus=DEACTIVATED&limit=20&offset=0")).toEqual([
      { name: "qrStatus", value: "DEACTIVATED" },
      { name: "limit", value: "20" },
      { name: "offset", value: "0" },
    ]);
  });

  it("returns nothing for an empty query", () => {
    expect(paramsFromQuery("")).toEqual([]);
    expect(paramsFromQuery("?")).toEqual([]);
  });

  it("tolerates a leading question mark", () => {
    expect(paramsFromQuery("?a=1")).toEqual([{ name: "a", value: "1" }]);
  });

  it("treats a bare key as an empty value, like the engine", () => {
    expect(paramsFromQuery("flag&a=")).toEqual([
      { name: "flag", value: "" },
      { name: "a", value: "" },
    ]);
  });

  it("keeps repeated keys as separate rows", () => {
    expect(paramsFromQuery("id=1&id=2")).toEqual([
      { name: "id", value: "1" },
      { name: "id", value: "2" },
    ]);
  });

  it("decodes + and percent escapes, including UTF-8", () => {
    expect(paramsFromQuery("q=a+b&p=a%20b&t=%D0%9F%D0%BE%D0%BC%D0%BE%D1%89%D1%8C")).toEqual([
      { name: "q", value: "a b" },
      { name: "p", value: "a b" },
      { name: "t", value: "Помощь" },
    ]);
  });

  it("decodes an escaped plus and an escaped percent like the engine does", () => {
    // The engine decodes in one pass, so a byte produced by an escape is never
    // re-examined: `%2B` is a literal "+", `%2520` is a literal "%20".
    expect(paramsFromQuery("a=%2B&b=%2520")).toEqual([
      { name: "a", value: "+" },
      { name: "b", value: "%20" },
    ]);
  });

  it("leaves a malformed escape alone instead of throwing", () => {
    expect(paramsFromQuery("a=%zz")).toEqual([{ name: "a", value: "%zz" }]);
  });

  it("drops empty segments and nameless pairs", () => {
    expect(paramsFromQuery("a=1&&=2&b=3")).toEqual([
      { name: "a", value: "1" },
      { name: "b", value: "3" },
    ]);
  });

  it("keeps a value that itself contains =", () => {
    expect(paramsFromQuery("token=ab=cd")).toEqual([{ name: "token", value: "ab=cd" }]);
  });
});

describe("splitPathQuery", () => {
  it("splits on the first question mark", () => {
    expect(splitPathQuery("/v1/getpaylinks?limit=20")).toEqual({
      path: "/v1/getpaylinks",
      query: "limit=20",
    });
  });

  it("leaves a query-less path whole", () => {
    expect(splitPathQuery("/v1/getpaylinks")).toEqual({
      path: "/v1/getpaylinks",
      query: "",
    });
  });
});
