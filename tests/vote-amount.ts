import { strict as assert } from "node:assert";
import { PublicKey } from "@solana/web3.js";
import { MIN_VOTE_AMOUNT, voteIx } from "../lib/vote";

const address = new PublicKey("11111111111111111111111111111111");

assert.throws(
  () => voteIx(address, address, address, address, 0, MIN_VOTE_AMOUNT - 1n),
  /至少需要 0\.01 CVOTE/,
);

const instruction = voteIx(address, address, address, address, 0, MIN_VOTE_AMOUNT);
assert.equal(instruction.data.readBigUInt64LE(2), 10_000n);

console.log("vote amount boundary: ok");
