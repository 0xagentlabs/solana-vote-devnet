import { readFileSync } from "node:fs";
import { Connection, Keypair, Transaction, sendAndConfirmTransaction } from "@solana/web3.js";
import { REWARD_STATE, initializeRewardsIx, parseRewardState } from "../lib/vote";

async function main() {
  const connection = new Connection("https://api.devnet.solana.com", "confirmed");
  const payerPath = process.env.SOLANA_KEYPAIR ?? `${process.env.HOME}/.config/solana/id.json`;
  const payer = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(payerPath, "utf8"))));
  const existing = await connection.getAccountInfo(REWARD_STATE);
  if (existing) {
    console.log(JSON.stringify({ rewardState: REWARD_STATE.toBase58(), state: parseRewardState(existing.data), alreadyInitialized: true }, (_, value) => typeof value === "bigint" ? value.toString() : value, 2));
    return;
  }
  const transaction = new Transaction().add(initializeRewardsIx(payer.publicKey));
  const simulation = await connection.simulateTransaction(transaction, [payer]);
  if (simulation.value.err) throw new Error(`Simulation failed: ${JSON.stringify(simulation.value.err)}\n${simulation.value.logs?.join("\n")}`);
  const signature = await sendAndConfirmTransaction(connection, transaction, [payer], { commitment: "confirmed" });
  const account = await connection.getAccountInfo(REWARD_STATE);
  if (!account) throw new Error("RewardState was not created");
  console.log(JSON.stringify({ rewardState: REWARD_STATE.toBase58(), state: parseRewardState(account.data), signature }, (_, value) => typeof value === "bigint" ? value.toString() : value, 2));
}

main().catch(error => { console.error(error); process.exit(1); });
