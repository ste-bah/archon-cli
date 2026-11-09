import { test } from "node:test";
import assert from "node:assert/strict";
import { add } from "./proof.ts";
test("failing addition", () => assert.equal(add(1, 1), 3));
