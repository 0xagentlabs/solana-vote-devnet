import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import { Connection, Keypair, Transaction, sendAndConfirmTransaction } from "@solana/web3.js";
import { createAssociatedTokenAccountInstruction, getAssociatedTokenAddressSync } from "@solana/spl-token";
import { CONFIG, PROGRAM_ID, claimIx, createProposalIx, joinIx, memberPda, parseConfig, parseProposal, proposalPda } from "../lib/vote";

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
  const content = "是否采用链上提案内容与实时计票界面？";
  const proposalSignature = await sendAndConfirmTransaction(connection, new Transaction().add(createAssociatedTokenAccountInstruction(payer.publicKey, vault, proposal, config.mint), createProposalIx(payer.publicKey, nonce, vault, content)), [payer], { commitment: "confirmed" });
  const proposalAccount = await connection.getAccountInfo(proposal);
  assert(proposalAccount);
  const parsed = parseProposal(proposalAccount.data);
  assert.equal(parsed.content, content);
  assert.equal(parsed.yes, 0n);
  assert.equal(parsed.no, 0n);
  assert(parsed.endAt > Math.floor(Date.now() / 1000));
  console.log(JSON.stringify({ joinSignature, proposalSignature, proposal: proposal.toBase58(), claimSimulation: "ok", content: parsed.content }));
}

main().catch(error => { console.error(error); process.exit(1); });
