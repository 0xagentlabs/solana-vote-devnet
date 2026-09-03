import {readFileSync} from "node:fs";
import {Connection,Keypair,PublicKey,SystemProgram,Transaction,TransactionInstruction,sendAndConfirmTransaction} from "@solana/web3.js";
import {createAssociatedTokenAccountInstruction,createInitializeMintInstruction,getAssociatedTokenAddressSync,MINT_SIZE,TOKEN_PROGRAM_ID} from "@solana/spl-token";
import {CONFIG,PROGRAM_ID} from "../lib/vote";

async function main(){
const rpc="https://api.devnet.solana.com";
const payerPath=process.env.SOLANA_KEYPAIR ?? `${process.env.HOME}/.config/solana/id.json`;
const payer=Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(payerPath,"utf8"))));
const connection=new Connection(rpc,"confirmed");
if(await connection.getAccountInfo(CONFIG)) throw new Error("Config already initialized");
const mint=Keypair.generate();
const treasury=getAssociatedTokenAddressSync(mint.publicKey,CONFIG,true);
const rent=await connection.getMinimumBalanceForRentExemption(MINT_SIZE);
const total=1_000_000_000_000n;
const data=Buffer.alloc(10);data[0]=0;data[1]=6;data.writeBigUInt64LE(total,2);
const initialize=new TransactionInstruction({programId:PROGRAM_ID,keys:[
  {pubkey:payer.publicKey,isSigner:true,isWritable:true},{pubkey:CONFIG,isSigner:false,isWritable:true},
  {pubkey:mint.publicKey,isSigner:false,isWritable:true},{pubkey:treasury,isSigner:false,isWritable:true},
  {pubkey:TOKEN_PROGRAM_ID,isSigner:false,isWritable:false},{pubkey:SystemProgram.programId,isSigner:false,isWritable:false}],data});
const tx=new Transaction().add(SystemProgram.createAccount({fromPubkey:payer.publicKey,newAccountPubkey:mint.publicKey,lamports:rent,space:MINT_SIZE,programId:TOKEN_PROGRAM_ID}),createInitializeMintInstruction(mint.publicKey,6,CONFIG,null),createAssociatedTokenAccountInstruction(payer.publicKey,treasury,CONFIG,mint.publicKey),initialize);
const simulation=await connection.simulateTransaction(tx,[payer,mint]);if(simulation.value.err)throw new Error(`Simulation failed: ${JSON.stringify(simulation.value.err)}\n${simulation.value.logs?.join("\n")}`);
const signature=await sendAndConfirmTransaction(connection,tx,[payer,mint],{commitment:"confirmed"});
console.log(JSON.stringify({programId:PROGRAM_ID.toBase58(),config:CONFIG.toBase58(),mint:mint.publicKey.toBase58(),treasury:treasury.toBase58(),signature},null,2));
}
main().catch(error=>{console.error(error);process.exit(1)});
