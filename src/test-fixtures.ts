/**
 * Shapes that tests need whole but only care about a field or two of.
 *
 * A group is the first of them. Five test files each built one by hand, so
 * adding a setting to a group meant editing five fixtures that had nothing to
 * do with the change and nothing to say about it. Imported only by tests, so
 * nothing here reaches the bundle.
 */

import type { Group, GroupId } from "./lib/types";

/** The group the migration creates, which the UI hides while it is the only one. */
export const DEFAULT_GROUP: GroupId = "00000000-0000-4000-8000-000000000001";

/** A group that inherits everything, with whatever the test cares about on top. */
export function aGroup(over: Partial<Group> = {}): Group {
  return {
    id: DEFAULT_GROUP,
    name: "Everyone",
    agentCount: 0,
    createdAt: 0,
    inference: {
      provider: null,
      baseUrl: null,
      defaultModel: null,
      subscriptionModel: null,
      requestTimeoutSecs: null,
    },
    apiKeySet: false,
    apiKeyHint: "",
    limits: {
      maxHops: null,
      maxStepsPerRun: null,
      maxFanoutPerCall: null,
      maxSendsPerPair: null,
      maxToolRounds: null,
    },
    ...over,
  };
}
