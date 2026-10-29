import { describe, expect, it } from "vitest";

import { account } from "../test/fixtures";
import {
  changedProviders,
  loginCommand,
  needsLogin,
  stateDetail,
  stateLabel,
} from "./accounts";

describe("accounts", () => {
  it("fills the host into the login command, or leaves it out", () => {
    expect(loginCommand("github", " ghe.example.test ")).toBe(
      "gh auth login --hostname ghe.example.test",
    );
    expect(loginCommand("gitlab", "")).toBe("glab auth login");
  });

  it("says what a row's state is", () => {
    expect(stateLabel(account())).toEqual({
      text: "Logged in as rev-fixture",
      tone: "ok",
    });
    const missing = account({
      provider: "gitlab",
      host: "",
      state: { kind: "cliMissing", hint: "Install glab." },
    });
    expect(stateLabel(missing)).toEqual({
      text: "glab not found",
      tone: "warning",
    });
    expect(stateDetail(missing)).toBe("Install glab.");
    expect(needsLogin(missing)).toBe(true);
    expect(needsLogin(account())).toBe(false);
  });

  it("names the providers whose entries a patch changes", () => {
    const off = { enabled: false, account: null };
    expect(changedProviders({}, { "gitlab:gl.example.test": off })).toEqual([
      "gitlab",
    ]);
    expect(
      changedProviders(
        { "github:github.com": off },
        { "github:github.com": off },
      ),
    ).toEqual([]);
    expect(
      changedProviders(
        { "github:github.com": off },
        { "github:github.com": { enabled: false, account: "work" } },
      ),
    ).toEqual(["github"]);
  });
});
