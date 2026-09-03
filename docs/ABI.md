# Civic Vote ABI

Program ID: `BvTUyzWLwyoX47bXndjupDzzbtHBFGNWUsZhwsFNkTXe`. 所有整数均为 little-endian，时间为 Unix 秒，Token 精度固定 6。该文档是 Pinocchio 项目的自有 ABI，不是 Anchor IDL。

## PDA 与状态

- Config：`["config"]`，96 bytes。`disc:u8=1, bump:u8, decimals:u8, reserved[5], mint:pubkey, treasury:pubkey, total:u64, allocated:u64, recovered:u64`。
- Member：`["member", wallet]`，80 bytes。`disc=2, bump, reserved[6], wallet, joined_at:i64, last_claim_at:i64, allocation:u64, claimed:u64, reserved[8]`。
- Proposal：`["proposal", creator, nonce_le_u64]`，120 bytes。`disc=3, bump, settled:u8, result:u8, reserved[4], creator, end_at:i64, yes:u64, no:u64, deposited:u64, nonce:u64, vault:pubkey, reserved[8]`。
- VoteReceipt：`["vote", proposal, voter]`，56 bytes。`disc=4, bump, option:u8, reserved[5], voter, amount:u64, reserved[8]`。

## 指令

| Tag | 参数 | Accounts（按顺序） |
|---|---|---|
| 0 Initialize | `decimals:u8=6,total:u64=1_000_000e6` | payer(s,w), config(w), mint(w), treasury(w), Token Program, System Program |
| 1 Join | 无 | user(s,w), member(w), config(w), System Program |
| 2 Claim | 无 | user(s), member(w), config, treasury(w), user token(w), Token Program |
| 3 CreateProposal | `nonce:u64` | creator(s,w), proposal(w), proposal vault, config, System Program |
| 4 Vote | `option:u8 (0=yes,1=no),amount:u64` | voter(s,w), proposal(w), receipt(w), voter token(w), proposal vault(w), config, System Program, Token Program |
| 5 Settle | 无 | proposal(w), proposal vault(w), config, treasury(w), Token Program |

Initialize 铸造全部供应到 treasury 后立即撤销 mint authority；Join 每钱包仅能初始化一次并预留 1,000 Token；Claim 按 `min(elapsed,365d)/365d` 线性释放且间隔至少 24h；Proposal 固定 7 天；同一钱包只可选择一个方向但可追加；任何人可在到期后结算，全部投票 Token 回到 treasury。

## 错误码

`1 InvalidAccounts`, `2 InvalidPda`, `3 InvalidState`, `4 InvalidMint`, `5 InvalidAmount`, `6 MathOverflow`, `7 TooEarly`, `8 VotingClosed`, `9 AlreadySettled`, `10 AllocationExhausted`, `11 InvalidOption`。

