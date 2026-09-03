import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import { Connection, Keypair, Transaction, sendAndConfirmTransaction } from "@solana/web3.js";
import { createAssociatedTokenAccountInstruction, getAssociatedTokenAddressSync } from "@solana/spl-token";
import { CONFIG, PROGRAM_ID, PROPOSAL_FEE, claimIx, createProposalIx, joinIx, memberPda, parseConfig, proposalPda } from "../lib/vote";

async function main() {
  const connection = new Connection("https://api.devnet.solana.com", "confirmed");
  const payer = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(process.env.SOLANA_KEYPAIR ?? `${process.env.HOME}/.config/solana/id.json`, "utf8"))));
  assert((await connection.getAccountInfo(PROGRAM_ID))?.executable);
  const configAccount = await connection.getAccountInfo(CONFIG);
  assert(configAccount);
  const config = parseConfig(configAccount.data);
  assert.equal(config.total, 1_000_000_000_000n);
  const member = memberPda(payer.publicKey);
  let joinSignature: string | undefined;
  if (!await connection.getAccountInfo(member)) joinSignature = await sendAndConfirmTransaction(connection, new Transaction().add(joinIx(payer.publicKey)), [payer], { commitment: "confirmed" });
  assert.equal((await connection.getAccountInfo(member))?.owner.toBase58(), PROGRAM_ID.toBase58());

  const ata = getAssociatedTokenAddressSync(config.mint, payer.publicKey);
  const claim = new Transaction();
  if (!await connection.getAccountInfo(ata)) claim.add(createAssociatedTokenAccountInstruction(payer.publicKey, ata, payer.publicKey, config.mint));
  claim.add(claimIx(payer.publicKey, config.treasury, ata));
  assert.equal((await connection.simulateTransaction(claim, [payer])).value.err, null);

  const nonce = BigInt(Date.now());
  const proposal = proposalPda(payer.publicKey, nonce);
  const vault = getAssociatedTokenAddressSync(config.mint, proposal, true);
  let balance = 0n;
  try { balance = BigInt((await connection.getTokenAccountBalance(ata)).value.amount); } catch { /* ATA can be absent before the first claim. */ }
  const create = new Transaction();
  if (!await connection.getAccountInfo(ata)) create.add(createAssociatedTokenAccountInstruction(payer.publicKey, ata, payer.publicKey, config.mint));
  create.add(createAssociatedTokenAccountInstruction(payer.publicKey, vault, proposal, config.mint), createProposalIx(payer.publicKey, nonce, vault, ata, config.treasury, "10 CVOTE 创建费用验证"));
  const createSimulation = await connection.simulateTransaction(create, [payer]);
  if (balance >= PROPOSAL_FEE) assert.equal(createSimulation.value.err, null, JSON.stringify(createSimulation.value.logs));
  else assert(createSimulation.value.err, "Insufficient-token proposal creation must fail");
  console.log(JSON.stringify({ joinSignature, claimSimulation: "ok", proposalFee: PROPOSAL_FEE.toString(), walletBalance: balance.toString(), createSimulation: balance >= PROPOSAL_FEE ? "ok" : "correctly rejected: insufficient CVOTE" }));
}

main().catch(error => { console.error(error); process.exit(1); });
