import { it, expect } from "vitest";
import { eligibleProfileTargets } from "./ProfileCopy";
it("trusts backend compatibility, includes offline devices, and excludes source and unknown eligibility", () => {
  const source = {
    id: "A",
    model: "plus",
    name: "Source",
    connected: true,
    firmware: "1",
    selected: true,
    profile_compatible: true,
  };
  const offline = {
    ...source,
    id: "B",
    model: "different-model-approved-by-host",
    name: "Offline target",
    connected: false,
    selected: false,
  };
  const incompatible = { ...source, id: "C", profile_compatible: false };
  const unknown = { ...source, id: "D", profile_compatible: undefined };
  expect(
    eligibleProfileTargets([source, offline, incompatible, unknown], "A"),
  ).toEqual([offline]);
});
