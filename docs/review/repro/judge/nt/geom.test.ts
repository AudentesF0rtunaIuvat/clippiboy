import { test } from "node:test";
import assert from "node:assert/strict";
import { outward } from "./geom.ts";
test("outward covers the float rect", () => {
  assert.deepEqual(outward(10.4, 2.6, 15.8, 7.1), { x: 10, y: 2, width: 6, height: 6 });
});
