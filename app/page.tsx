"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { useConnection, useWallet } from "@solana/wallet-adapter-react";
import { WalletMultiButton } from "@solana/wallet-adapter-react-ui";
import { PublicKey, Transaction } from "@solana/web3.js";
import { createAssociatedTokenAccountInstruction, getAssociatedTokenAddressSync } from "@solana/spl-token";
import { CheckCircle2, Clock3, Landmark, ShieldCheck, Vote } from "lucide-react";
import { CONFIG, claimable, claimIx, createProposalIx, joinIx, memberPda, parseConfig, parseMember, parseProposal, proposalPda, settleIx, voteIx, type Config, type Member, type Proposal } from "../lib/vote";

const TOKENS = 1_000_000n;
const fmt = (value: bigint) => `${(Number(value) / Number(TOKENS)).toLocaleString("zh-CN", { maximumFractionDigits: 6 })} CVOTE`;
const date = (seconds: number) => new Intl.DateTimeFormat("zh-CN", { dateStyle: "medium", timeStyle: "short" }).format(seconds * 1000);

export default function Home() {
  const { connection } = useConnection();
  const wallet = useWallet();
  const [cfg, setCfg] = useState<Config>();
  const [member, setMember] = useState<Member>();
  const [proposalState, setProposalState] = useState<Proposal>();
  const [status, setStatus] = useState("连接钱包开始参与");
  const [busy, setBusy] = useState(false);
  const [proposal, setProposal] = useState("");
  const [proposalContent, setProposalContent] = useState("");
  const [amount, setAmount] = useState("1");
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));

  const load = useCallback(async () => {
    const configAccount = await connection.getAccountInfo(CONFIG);
    if (configAccount) setCfg(parseConfig(configAccount.data));
    if (wallet.publicKey) {
      const memberAccount = await connection.getAccountInfo(memberPda(wallet.publicKey));
      setMember(memberAccount ? parseMember(memberAccount.data) : undefined);
    } else setMember(undefined);
    if (proposal) {
      try {
        const proposalAccount = await connection.getAccountInfo(new PublicKey(proposal));
        setProposalState(proposalAccount ? parseProposal(proposalAccount.data) : undefined);
      } catch { setProposalState(undefined); }
    } else setProposalState(undefined);
  }, [connection, proposal, wallet.publicKey]);

  useEffect(() => { void load(); }, [load]);
  useEffect(() => { const id = window.setInterval(() => setNow(Math.floor(Date.now() / 1000)), 1000); return () => window.clearInterval(id); }, []);

  const available = useMemo(() => member ? claimable(member, now) : 0n, [member, now]);
  const claimReady = !!member && available > 0n && (member.lastClaimAt === 0 || now >= member.lastClaimAt + 24 * 60 * 60);
  const proposalStatus = proposalState ? proposalState.settled ? `已结算 · ${proposalState.result === 0 ? "赞成" : "反对"}胜出` : now >= proposalState.endAt ? "已到期 · 可结算回收" : "投票进行中" : undefined;

  const send = async (tx: Transaction, label: string) => {
    if (!wallet.publicKey) throw Error("请先连接钱包");
    setBusy(true);
    try {
      const latest = await connection.getLatestBlockhash();
      tx.feePayer = wallet.publicKey;
      tx.recentBlockhash = latest.blockhash;
      const sim = await connection.simulateTransaction(tx);
      if (sim.value.err) throw Error(`模拟失败：${JSON.stringify(sim.value.err)}`);
      const signature = await wallet.sendTransaction(tx, connection);
      await connection.confirmTransaction({ ...latest, signature }, "confirmed");
      setStatus(`${label}成功 · ${signature.slice(0, 12)}…`);
      await load();
    } catch (error) { setStatus(error instanceof Error ? error.message : "交易失败，请重试"); }
    finally { setBusy(false); }
  };

  const join = async () => { if (wallet.publicKey) await send(new Transaction().add(joinIx(wallet.publicKey)), "加入社区"); };
  const claim = async () => {
    if (!wallet.publicKey || !cfg) return;
    const ata = getAssociatedTokenAddressSync(cfg.mint, wallet.publicKey);
    const tx = new Transaction();
    if (!await connection.getAccountInfo(ata)) tx.add(createAssociatedTokenAccountInstruction(wallet.publicKey, ata, wallet.publicKey, cfg.mint));
    tx.add(claimIx(wallet.publicKey, cfg.treasury, ata));
    await send(tx, "领取");
  };
  const create = async () => {
    if (!wallet.publicKey || !cfg || !proposalContent.trim()) return;
    const nonce = BigInt(Date.now());
    const pda = proposalPda(wallet.publicKey, nonce);
    const vault = getAssociatedTokenAddressSync(cfg.mint, pda, true);
    await send(new Transaction().add(createAssociatedTokenAccountInstruction(wallet.publicKey, vault, pda, cfg.mint), createProposalIx(wallet.publicKey, nonce, vault, proposalContent)), "创建提案");
    setProposal(pda.toBase58());
  };
  const cast = async (option: number) => {
    if (!wallet.publicKey || !cfg || !proposalState) return;
    const pda = new PublicKey(proposal);
    const source = getAssociatedTokenAddressSync(cfg.mint, wallet.publicKey);
    await send(new Transaction().add(voteIx(wallet.publicKey, pda, source, proposalState.vault, option, BigInt(Math.round(Number(amount) * 1e6)))), "投票");
  };
  const settle = async () => {
    if (!cfg || !proposalState) return;
    await send(new Transaction().add(settleIx(new PublicKey(proposal), proposalState.vault, cfg.treasury)), "结算并回收");
  };

  return <main>
    <nav><a className="brand" href="#top"><Vote aria-hidden />CIVIC VOTE</a><span className="net">DEVNET</span><WalletMultiButton /></nav>
    <section id="top" className="hero"><p className="eyebrow">PERMISSIONLESS GOVERNANCE</p><h1>把加入时间，<br /><em>变成治理权。</em></h1><p>固定 1,000,000 枚 SPL Token。无管理员、无增发、链上线性释放，投票后份额回归社区 treasury。</p><div className="stats"><div><strong>1M</strong><span>固定供应</span></div><div><strong>365D</strong><span>线性释放</span></div><div><strong>7D</strong><span>投票周期</span></div></div></section>
    <section className="grid">
      <article><Landmark aria-hidden /><h2>加入社区</h2><p>首次加入自动锁定 1,000 枚额度，从链上时间开始释放。</p><div className={`state ${member ? "success" : ""}`}>{member ? `已加入 · ${date(member.joinedAt)}` : "尚未加入"}</div><button disabled={busy || !wallet.publicKey || !!member} onClick={join}>{member ? "已加入" : "加入社区"}</button></article>
      <article><Clock3 aria-hidden /><h2>领取份额</h2><p>每 24 小时领取已释放 Token，直至 365 天完全释放。</p><div className="balance"><span>当前可领取</span><strong>{fmt(available)}</strong><small>{!member ? "加入社区后开始释放" : member.lastClaimAt && !claimReady ? `下次可领取：${date(member.lastClaimAt + 24 * 60 * 60)}` : `累计已领取 ${fmt(member.claimed)}`}</small></div><button disabled={busy || !cfg || !claimReady} onClick={claim}>Claim {available > 0n ? fmt(available) : ""}</button></article>
      <article className="wide"><ShieldCheck aria-hidden /><h2>创建与参与提案</h2><p>明确写下要表决的事项。提案链上保存 7 天，投入份额后不可撤回，结算时回到社区 treasury。</p><div className="create-form"><label htmlFor="proposal-content">投票内容<textarea id="proposal-content" value={proposalContent} onChange={event => setProposalContent(event.target.value)} maxLength={80} placeholder="例如：是否将下一轮社区活动主题定为公共物品？" /><small>{new TextEncoder().encode(proposalContent.trim()).length}/160 UTF-8 字节</small></label><button disabled={busy || !cfg || !proposalContent.trim() || new TextEncoder().encode(proposalContent.trim()).length > 160} onClick={create}>创建 7 天提案</button></div>
        <div className="proposal-lookup"><label htmlFor="proposal-address">提案地址<input id="proposal-address" value={proposal} onChange={event => setProposal(event.target.value.trim())} placeholder="Proposal PDA" /></label></div>
        {proposalState ? <section className="proposal-card" aria-live="polite"><div className="proposal-heading"><div><span className="state">{proposalStatus}</span><h3>{proposalState.content}</h3></div><time dateTime={new Date(proposalState.endAt * 1000).toISOString()}>截止：{date(proposalState.endAt)}</time></div><div className="tally"><div><span>赞成</span><strong>{fmt(proposalState.yes)}</strong></div><div><span>反对</span><strong>{fmt(proposalState.no)}</strong></div></div><div className="vote-actions"><label htmlFor="vote-amount">投票数量<input id="vote-amount" type="number" min="0.000001" step="1" value={amount} onChange={event => setAmount(event.target.value)} /></label><button disabled={busy || proposalState.settled || now >= proposalState.endAt} onClick={() => cast(0)}>投赞成</button><button className="secondary" disabled={busy || proposalState.settled || now >= proposalState.endAt} onClick={() => cast(1)}>投反对</button><button className="ghost" disabled={busy || proposalState.settled || now < proposalState.endAt} onClick={settle}>结算并回收</button></div></section> : proposal && <p className="empty">未找到有效提案，请检查地址。</p>}
      </article>
    </section>
    <aside aria-live="polite"><CheckCircle2 aria-hidden />{busy ? "正在模拟并等待签名…" : status}</aside>
    <footer>Program <a href="https://explorer.solana.com/address/BvTUyzWLwyoX47bXndjupDzzbtHBFGNWUsZhwsFNkTXe?cluster=devnet" target="_blank" rel="noreferrer">BvTU…NTXe</a></footer>
  </main>;
}
