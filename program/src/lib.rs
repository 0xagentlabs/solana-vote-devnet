#![no_std]
#![allow(unexpected_cfgs)]
#![allow(clippy::possible_missing_else)]

use pinocchio::{
    account_info::AccountInfo,
    entrypoint,
    instruction::{Seed, Signer},
    program_error::ProgramError,
    pubkey::{find_program_address, Pubkey},
    sysvars::{clock::Clock, rent::Rent, Sysvar},
    ProgramResult,
};
use pinocchio_system::instructions::CreateAccount;
use pinocchio_token::{
    instructions::{AuthorityType, MintTo, SetAuthority, Transfer},
    state::{Mint, TokenAccount},
};

entrypoint!(process_instruction);
pinocchio::nostd_panic_handler!();

pub const ID: Pubkey = [
    0xa2, 0x46, 0x98, 0x90, 0x95, 0x22, 0xf7, 0xcf, 0xa0, 0xcc, 0xaa, 0x3f, 0x62, 0x7d, 0x9c, 0xdc,
    0x1f, 0x7a, 0x5d, 0x47, 0x4c, 0xa7, 0xd1, 0xb5, 0x27, 0x8e, 0xc5, 0xe2, 0xc0, 0xe1, 0x49, 0xe1,
];
const CONFIG_LEN: usize = 96;
const MEMBER_LEN: usize = 80;
const PROPOSAL_LEN: usize = 120;
const RECEIPT_LEN: usize = 56;
const CONFIG_DISC: u8 = 1;
const MEMBER_DISC: u8 = 2;
const PROPOSAL_DISC: u8 = 3;
const RECEIPT_DISC: u8 = 4;
const ALLOCATION: u64 = 1_000_000_000; // 1,000 tokens at 6 decimals.
const VESTING_SECONDS: i64 = 365 * 24 * 60 * 60;
const CLAIM_INTERVAL: i64 = 24 * 60 * 60;
const PROPOSAL_SECONDS: i64 = 7 * 24 * 60 * 60;

#[repr(u32)]
enum VoteError {
    InvalidAccounts = 1,
    InvalidPda,
    InvalidState,
    InvalidMint,
    InvalidAmount,
    MathOverflow,
    TooEarly,
    VotingClosed,
    AlreadySettled,
    AllocationExhausted,
    InvalidOption,
}
impl From<VoteError> for ProgramError {
    fn from(e: VoteError) -> Self {
        ProgramError::Custom(e as u32)
    }
}

fn process_instruction(program_id: &Pubkey, a: &[AccountInfo], data: &[u8]) -> ProgramResult {
    if program_id != &ID {
        return Err(ProgramError::IncorrectProgramId);
    }
    let (tag, d) = data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;
    match tag {
        0 => initialize(a, d),
        1 => join(a, d),
        2 => claim(a, d),
        3 => create_proposal(a, d),
        4 => vote(a, d),
        5 => settle(a, d),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

fn initialize(a: &[AccountInfo], d: &[u8]) -> ProgramResult {
    if a.len() != 6 || d.len() != 9 {
        return Err(VoteError::InvalidAccounts.into());
    }
    let (payer, config, mint, treasury, tp, sp) = (&a[0], &a[1], &a[2], &a[3], &a[4], &a[5]);
    if !payer.is_signer() || tp.key() != &pinocchio_token::ID || sp.key() != &pinocchio_system::ID {
        return Err(VoteError::InvalidAccounts.into());
    }
    let (expected, bump) = find_program_address(&[b"config"], &ID);
    if config.key() != &expected {
        return Err(VoteError::InvalidPda.into());
    }
    let total = read_u64(&d[1..9]);
    if d[0] != 6 || total != 1_000_000_000_000 {
        return Err(VoteError::InvalidAmount.into());
    }
    {
        let m = Mint::from_account_info(mint)?;
        if m.supply() != 0 || m.decimals() != 6 || m.mint_authority() != Some(config.key()) {
            return Err(VoteError::InvalidMint.into());
        }
        let t = TokenAccount::from_account_info(treasury)?;
        if t.mint() != mint.key() || t.owner() != config.key() {
            return Err(VoteError::InvalidMint.into());
        }
    }
    let bump_seed = [bump];
    let seeds = [Seed::from(b"config"), Seed::from(&bump_seed)];
    CreateAccount {
        from: payer,
        to: config,
        lamports: rent_lamports(CONFIG_LEN)?,
        space: CONFIG_LEN as u64,
        owner: &ID,
    }
    .invoke_signed(&[Signer::from(&seeds)])?;
    {
        let mut out = config.try_borrow_mut_data()?;
        out[0] = CONFIG_DISC;
        out[1] = bump;
        out[2] = 6;
        out[8..40].copy_from_slice(mint.key());
        out[40..72].copy_from_slice(treasury.key());
        out[72..80].copy_from_slice(&total.to_le_bytes());
    }
    MintTo {
        mint,
        account: treasury,
        mint_authority: config,
        amount: total,
    }
    .invoke_signed(&[Signer::from(&seeds)])?;
    SetAuthority {
        account: mint,
        authority: config,
        authority_type: AuthorityType::MintTokens,
        new_authority: None,
    }
    .invoke_signed(&[Signer::from(&seeds)])
}

fn join(a: &[AccountInfo], d: &[u8]) -> ProgramResult {
    if a.len() != 4 || !d.is_empty() {
        return Err(VoteError::InvalidAccounts.into());
    }
    let (user, member, config, sp) = (&a[0], &a[1], &a[2], &a[3]);
    if !user.is_signer() || sp.key() != &pinocchio_system::ID {
        return Err(VoteError::InvalidAccounts.into());
    }
    let cb = validate_config(config)?;
    let (expected, bump) = find_program_address(&[b"member", user.key()], &ID);
    if member.key() != &expected {
        return Err(VoteError::InvalidPda.into());
    }
    let mut allocated = {
        let cd = config.try_borrow_data()?;
        read_u64(&cd[80..88])
    };
    allocated = allocated
        .checked_add(ALLOCATION)
        .ok_or(VoteError::MathOverflow)?;
    let total = {
        let cd = config.try_borrow_data()?;
        read_u64(&cd[72..80])
    };
    if allocated > total {
        return Err(VoteError::AllocationExhausted.into());
    }
    let bump_seed = [bump];
    let seeds = [
        Seed::from(b"member"),
        Seed::from(user.key()),
        Seed::from(&bump_seed),
    ];
    CreateAccount {
        from: user,
        to: member,
        lamports: rent_lamports(MEMBER_LEN)?,
        space: MEMBER_LEN as u64,
        owner: &ID,
    }
    .invoke_signed(&[Signer::from(&seeds)])?;
    let now = Clock::get()?.unix_timestamp;
    {
        let mut md = member.try_borrow_mut_data()?;
        md[0] = MEMBER_DISC;
        md[1] = bump;
        md[8..40].copy_from_slice(user.key());
        md[40..48].copy_from_slice(&now.to_le_bytes());
        md[56..64].copy_from_slice(&ALLOCATION.to_le_bytes());
    }
    let mut cd = config.try_borrow_mut_data()?;
    cd[80..88].copy_from_slice(&allocated.to_le_bytes());
    cd[1] = cb;
    Ok(())
}

fn claim(a: &[AccountInfo], d: &[u8]) -> ProgramResult {
    if a.len() != 6 || !d.is_empty() {
        return Err(VoteError::InvalidAccounts.into());
    }
    let (user, member, config, treasury, dest, tp) = (&a[0], &a[1], &a[2], &a[3], &a[4], &a[5]);
    if !user.is_signer() || tp.key() != &pinocchio_token::ID {
        return Err(VoteError::InvalidAccounts.into());
    }
    let cb = validate_config(config)?;
    let cd = config.try_borrow_data()?;
    if treasury.key().as_ref() != &cd[40..72]
        || TokenAccount::from_account_info(treasury)?.owner() != config.key()
    {
        return Err(VoteError::InvalidMint.into());
    }
    drop(cd);
    let (joined, last, allocation, claimed) = {
        let md = member.try_borrow_data()?;
        if member.owner() != &ID
            || md.len() != MEMBER_LEN
            || md[0] != MEMBER_DISC
            || user.key().as_ref() != &md[8..40]
        {
            return Err(VoteError::InvalidState.into());
        }
        (
            read_i64(&md[40..48]),
            read_i64(&md[48..56]),
            read_u64(&md[56..64]),
            read_u64(&md[64..72]),
        )
    };
    let now = Clock::get()?.unix_timestamp;
    if last != 0 && now - last < CLAIM_INTERVAL {
        return Err(VoteError::TooEarly.into());
    }
    let elapsed = (now - joined).clamp(0, VESTING_SECONDS) as u128;
    let vested = ((allocation as u128)
        .checked_mul(elapsed)
        .ok_or(VoteError::MathOverflow)?
        / (VESTING_SECONDS as u128)) as u64;
    let amount = vested.checked_sub(claimed).ok_or(VoteError::MathOverflow)?;
    if amount == 0 {
        return Err(VoteError::InvalidAmount.into());
    }
    {
        let dst = TokenAccount::from_account_info(dest)?;
        if dst.owner() != user.key() {
            return Err(VoteError::InvalidMint.into());
        }
    }
    let bump_seed = [cb];
    let seeds = [Seed::from(b"config"), Seed::from(&bump_seed)];
    Transfer {
        from: treasury,
        to: dest,
        authority: config,
        amount,
    }
    .invoke_signed(&[Signer::from(&seeds)])?;
    let mut md = member.try_borrow_mut_data()?;
    md[48..56].copy_from_slice(&now.to_le_bytes());
    md[64..72].copy_from_slice(&vested.to_le_bytes());
    Ok(())
}

fn create_proposal(a: &[AccountInfo], d: &[u8]) -> ProgramResult {
    if a.len() != 5 || d.len() != 8 {
        return Err(VoteError::InvalidAccounts.into());
    }
    let (creator, proposal, vault, config, sp) = (&a[0], &a[1], &a[2], &a[3], &a[4]);
    if !creator.is_signer() || sp.key() != &pinocchio_system::ID {
        return Err(VoteError::InvalidAccounts.into());
    }
    validate_config(config)?;
    let nonce = read_u64(d);
    let (expected, bump) = find_program_address(&[b"proposal", creator.key(), d], &ID);
    if proposal.key() != &expected {
        return Err(VoteError::InvalidPda.into());
    }
    let cd = config.try_borrow_data()?;
    let mint = &cd[8..40];
    let vt = TokenAccount::from_account_info(vault)?;
    if vt.owner() != proposal.key() || vt.mint() != mint {
        return Err(VoteError::InvalidMint.into());
    }
    drop(cd);
    let bs = [bump];
    let seeds = [
        Seed::from(b"proposal"),
        Seed::from(creator.key()),
        Seed::from(d),
        Seed::from(&bs),
    ];
    CreateAccount {
        from: creator,
        to: proposal,
        lamports: rent_lamports(PROPOSAL_LEN)?,
        space: PROPOSAL_LEN as u64,
        owner: &ID,
    }
    .invoke_signed(&[Signer::from(&seeds)])?;
    let end = Clock::get()?
        .unix_timestamp
        .checked_add(PROPOSAL_SECONDS)
        .ok_or(VoteError::MathOverflow)?;
    let mut pd = proposal.try_borrow_mut_data()?;
    pd[0] = PROPOSAL_DISC;
    pd[1] = bump;
    pd[8..40].copy_from_slice(creator.key());
    pd[40..48].copy_from_slice(&end.to_le_bytes());
    pd[72..80].copy_from_slice(&nonce.to_le_bytes());
    pd[80..112].copy_from_slice(vault.key());
    Ok(())
}

fn vote(a: &[AccountInfo], d: &[u8]) -> ProgramResult {
    if a.len() != 8 || d.len() != 9 {
        return Err(VoteError::InvalidAccounts.into());
    }
    let option = d[0];
    let amount = read_u64(&d[1..]);
    if option > 1 || amount == 0 {
        return Err(VoteError::InvalidOption.into());
    }
    let (voter, proposal, receipt, source, vault, config, sp, tp) =
        (&a[0], &a[1], &a[2], &a[3], &a[4], &a[5], &a[6], &a[7]);
    if !voter.is_signer() || sp.key() != &pinocchio_system::ID || tp.key() != &pinocchio_token::ID {
        return Err(VoteError::InvalidAccounts.into());
    }
    validate_config(config)?;
    let (pb, end, settled, pvault) = {
        let pd = proposal.try_borrow_data()?;
        if proposal.owner() != &ID || pd.len() != PROPOSAL_LEN || pd[0] != PROPOSAL_DISC {
            return Err(VoteError::InvalidState.into());
        }
        (
            pd[1],
            read_i64(&pd[40..48]),
            pd[2],
            <[u8; 32]>::try_from(&pd[80..112]).unwrap(),
        )
    };
    if settled != 0 || Clock::get()?.unix_timestamp >= end {
        return Err(VoteError::VotingClosed.into());
    }
    if vault.key() != &pvault {
        return Err(VoteError::InvalidMint.into());
    }
    let (expected, rb) = find_program_address(&[b"vote", proposal.key(), voter.key()], &ID);
    if receipt.key() != &expected {
        return Err(VoteError::InvalidPda.into());
    }
    if receipt.data_len() == 0 {
        let bs = [rb];
        let seeds = [
            Seed::from(b"vote"),
            Seed::from(proposal.key()),
            Seed::from(voter.key()),
            Seed::from(&bs),
        ];
        CreateAccount {
            from: voter,
            to: receipt,
            lamports: rent_lamports(RECEIPT_LEN)?,
            space: RECEIPT_LEN as u64,
            owner: &ID,
        }
        .invoke_signed(&[Signer::from(&seeds)])?;
        let mut rd = receipt.try_borrow_mut_data()?;
        rd[0] = RECEIPT_DISC;
        rd[1] = rb;
        rd[2] = option;
        rd[8..40].copy_from_slice(voter.key());
    }
    {
        let rd = receipt.try_borrow_data()?;
        if rd[0] != RECEIPT_DISC || rd[2] != option || voter.key().as_ref() != &rd[8..40] {
            return Err(VoteError::InvalidState.into());
        }
    }
    Transfer {
        from: source,
        to: vault,
        authority: voter,
        amount,
    }
    .invoke()?;
    {
        let mut rd = receipt.try_borrow_mut_data()?;
        let n = read_u64(&rd[40..48])
            .checked_add(amount)
            .ok_or(VoteError::MathOverflow)?;
        rd[40..48].copy_from_slice(&n.to_le_bytes());
    }
    let mut pd = proposal.try_borrow_mut_data()?;
    let off = if option == 0 { 48 } else { 56 };
    let n = read_u64(&pd[off..off + 8])
        .checked_add(amount)
        .ok_or(VoteError::MathOverflow)?;
    pd[off..off + 8].copy_from_slice(&n.to_le_bytes());
    let dep = read_u64(&pd[64..72])
        .checked_add(amount)
        .ok_or(VoteError::MathOverflow)?;
    pd[64..72].copy_from_slice(&dep.to_le_bytes());
    pd[1] = pb;
    Ok(())
}

fn settle(a: &[AccountInfo], d: &[u8]) -> ProgramResult {
    if a.len() != 5 || !d.is_empty() {
        return Err(VoteError::InvalidAccounts.into());
    }
    let (proposal, vault, config, treasury, tp) = (&a[0], &a[1], &a[2], &a[3], &a[4]);
    if tp.key() != &pinocchio_token::ID {
        return Err(VoteError::InvalidAccounts.into());
    }
    let cb = validate_config(config)?;
    let (bump, creator, nonce, end, settled, deposited, pvault) = {
        let pd = proposal.try_borrow_data()?;
        if proposal.owner() != &ID || pd[0] != PROPOSAL_DISC {
            return Err(VoteError::InvalidState.into());
        }
        (
            pd[1],
            <[u8; 32]>::try_from(&pd[8..40]).unwrap(),
            read_u64(&pd[72..80]),
            read_i64(&pd[40..48]),
            pd[2],
            read_u64(&pd[64..72]),
            <[u8; 32]>::try_from(&pd[80..112]).unwrap(),
        )
    };
    if settled != 0 {
        return Err(VoteError::AlreadySettled.into());
    }
    if Clock::get()?.unix_timestamp < end {
        return Err(VoteError::TooEarly.into());
    }
    if vault.key() != &pvault {
        return Err(VoteError::InvalidMint.into());
    }
    let cd = config.try_borrow_data()?;
    if treasury.key().as_ref() != &cd[40..72] {
        return Err(VoteError::InvalidMint.into());
    }
    drop(cd);
    let nonceb = nonce.to_le_bytes();
    let bs = [bump];
    let seeds = [
        Seed::from(b"proposal"),
        Seed::from(creator.as_ref()),
        Seed::from(&nonceb),
        Seed::from(&bs),
    ];
    Transfer {
        from: vault,
        to: treasury,
        authority: proposal,
        amount: deposited,
    }
    .invoke_signed(&[Signer::from(&seeds)])?;
    let mut pd = proposal.try_borrow_mut_data()?;
    pd[2] = 1;
    pd[3] = if read_u64(&pd[48..56]) >= read_u64(&pd[56..64]) {
        0
    } else {
        1
    };
    let _ = cb;
    Ok(())
}

fn validate_config(c: &AccountInfo) -> Result<u8, ProgramError> {
    if c.owner() != &ID || c.data_len() != CONFIG_LEN {
        return Err(VoteError::InvalidState.into());
    }
    let d = c.try_borrow_data()?;
    if d[0] != CONFIG_DISC {
        return Err(VoteError::InvalidState.into());
    }
    let (p, _) = find_program_address(&[b"config"], &ID);
    if c.key() != &p {
        return Err(VoteError::InvalidPda.into());
    }
    Ok(d[1])
}
fn rent_lamports(len: usize) -> Result<u64, ProgramError> {
    Ok(Rent::get()?.minimum_balance(len))
}
fn read_u64(d: &[u8]) -> u64 {
    u64::from_le_bytes(d[..8].try_into().unwrap())
}
fn read_i64(d: &[u8]) -> i64 {
    i64::from_le_bytes(d[..8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn constants() {
        assert_eq!(ALLOCATION, 1_000_000_000);
        assert_eq!(VESTING_SECONDS, 31_536_000);
        assert_eq!(PROPOSAL_SECONDS, 604_800);
    }
    #[test]
    fn linear_math() {
        let vested = ALLOCATION as u128 * 15_768_000u128 / VESTING_SECONDS as u128;
        assert_eq!(vested, 500_000_000);
    }
}
