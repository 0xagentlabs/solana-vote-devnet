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
const LEGACY_PROPOSAL_LEN: usize = 120;
const PROPOSAL_LEN: usize = 280;
const MAX_PROPOSAL_CONTENT_LEN: usize = 160;
const RECEIPT_LEN: usize = 56;
const REWARD_STATE_LEN: usize = 64;
const REWARD_RECEIPT_LEN: usize = 56;
const CONFIG_DISC: u8 = 1;
const MEMBER_DISC: u8 = 2;
const PROPOSAL_DISC: u8 = 3;
const RECEIPT_DISC: u8 = 4;
const REWARD_STATE_DISC: u8 = 5;
const REWARD_RECEIPT_DISC: u8 = 6;
const ALLOCATION: u64 = 1_000_000_000; // 1,000 tokens at 6 decimals.
const VESTING_SECONDS: i64 = 365 * 24 * 60 * 60;
const CLAIM_INTERVAL: i64 = 24 * 60 * 60;
const PROPOSAL_SECONDS: i64 = 7 * 24 * 60 * 60;
const PROPOSAL_FEE: u64 = 10_000_000; // 10 tokens at 6 decimals, returned to treasury.
const MIN_VOTE_AMOUNT: u64 = 10_000; // 0.01 tokens at 6 decimals.

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
        6 => initialize_rewards(a, d),
        7 => claim_reward(a, d),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

fn initialize(a: &[AccountInfo], d: &[u8]) -> ProgramResult {
    if a.len() != 6 || d.len() != 9 {
        return Err(VoteError::InvalidAccounts.into());
    }
    let (payer, config, mint, treasury, tp, sp) = (&a[0], &a[1], &a[2], &a[3], &a[4], &a[5]);
    if !payer.is_signer()
        || !payer.is_writable()
        || !config.is_writable()
        || !mint.is_writable()
        || !treasury.is_writable()
        || tp.key() != &pinocchio_token::ID
        || sp.key() != &pinocchio_system::ID
    {
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
    if a.len() != 6 || !d.is_empty() {
        return Err(VoteError::InvalidAccounts.into());
    }
    let (user, member, reward_receipt, config, reward_state, sp) =
        (&a[0], &a[1], &a[2], &a[3], &a[4], &a[5]);
    if !user.is_signer()
        || !user.is_writable()
        || !member.is_writable()
        || !reward_receipt.is_writable()
        || !config.is_writable()
        || !reward_state.is_writable()
        || sp.key() != &pinocchio_system::ID
    {
        return Err(VoteError::InvalidAccounts.into());
    }
    let cb = validate_config(config)?;
    validate_reward_state(reward_state)?;
    let (expected, bump) = find_program_address(&[b"member", user.key()], &ID);
    if member.key() != &expected {
        return Err(VoteError::InvalidPda.into());
    }
    let (expected_reward, reward_bump) = find_program_address(&[b"reward", user.key()], &ID);
    if reward_receipt.key() != &expected_reward || reward_receipt.data_len() != 0 {
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
    let reward_bump_seed = [reward_bump];
    let reward_seeds = [
        Seed::from(b"reward"),
        Seed::from(user.key()),
        Seed::from(&reward_bump_seed),
    ];
    CreateAccount {
        from: user,
        to: reward_receipt,
        lamports: rent_lamports(REWARD_RECEIPT_LEN)?,
        space: REWARD_RECEIPT_LEN as u64,
        owner: &ID,
    }
    .invoke_signed(&[Signer::from(&reward_seeds)])?;
    let current_index = {
        let rd = reward_state.try_borrow_data()?;
        read_u64(&rd[16..24])
    };
    {
        let mut receipt_data = reward_receipt.try_borrow_mut_data()?;
        receipt_data[0] = REWARD_RECEIPT_DISC;
        receipt_data[1] = reward_bump;
        receipt_data[8..40].copy_from_slice(user.key());
        receipt_data[40..48].copy_from_slice(&current_index.to_le_bytes());
    }
    {
        let mut reward_data = reward_state.try_borrow_mut_data()?;
        let member_count = read_u64(&reward_data[8..16])
            .checked_add(1)
            .ok_or(VoteError::MathOverflow)?;
        reward_data[8..16].copy_from_slice(&member_count.to_le_bytes());
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
    if !user.is_signer()
        || !member.is_writable()
        || !treasury.is_writable()
        || !dest.is_writable()
        || tp.key() != &pinocchio_token::ID
    {
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
    if a.len() != 9 || d.len() < 10 {
        return Err(VoteError::InvalidAccounts.into());
    }
    let (creator, proposal, vault, config, reward_state, source, treasury, sp, tp) = (
        &a[0], &a[1], &a[2], &a[3], &a[4], &a[5], &a[6], &a[7], &a[8],
    );
    if !creator.is_signer()
        || !creator.is_writable()
        || !proposal.is_writable()
        || !config.is_writable()
        || !reward_state.is_writable()
        || !source.is_writable()
        || !treasury.is_writable()
        || sp.key() != &pinocchio_system::ID
        || tp.key() != &pinocchio_token::ID
    {
        return Err(VoteError::InvalidAccounts.into());
    }
    validate_config(config)?;
    validate_reward_state(reward_state)?;
    let nonce_bytes = &d[..8];
    let nonce = read_u64(nonce_bytes);
    let content_len = u16::from_le_bytes([d[8], d[9]]) as usize;
    if content_len == 0
        || content_len > MAX_PROPOSAL_CONTENT_LEN
        || d.len() != 10 + content_len
        || core::str::from_utf8(&d[10..]).is_err()
    {
        return Err(ProgramError::InvalidInstructionData);
    }
    let (expected, bump) = find_program_address(&[b"proposal", creator.key(), nonce_bytes], &ID);
    if proposal.key() != &expected {
        return Err(VoteError::InvalidPda.into());
    }
    let cd = config.try_borrow_data()?;
    let mint = &cd[8..40];
    if treasury.key().as_ref() != &cd[40..72] {
        return Err(VoteError::InvalidMint.into());
    }
    let vt = TokenAccount::from_account_info(vault)?;
    let src = TokenAccount::from_account_info(source)?;
    let dst = TokenAccount::from_account_info(treasury)?;
    if vt.owner() != proposal.key()
        || vt.mint() != mint
        || src.owner() != creator.key()
        || src.mint() != mint
        || dst.owner() != config.key()
        || dst.mint() != mint
    {
        return Err(VoteError::InvalidMint.into());
    }
    drop(cd);
    Transfer {
        from: source,
        to: treasury,
        authority: creator,
        amount: PROPOSAL_FEE,
    }
    .invoke()?;
    record_recovery(config, reward_state, PROPOSAL_FEE)?;
    let bs = [bump];
    let seeds = [
        Seed::from(b"proposal"),
        Seed::from(creator.key()),
        Seed::from(nonce_bytes),
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
    pd[112..114].copy_from_slice(&(content_len as u16).to_le_bytes());
    pd[114..114 + content_len].copy_from_slice(&d[10..]);
    Ok(())
}

fn vote(a: &[AccountInfo], d: &[u8]) -> ProgramResult {
    if a.len() != 8 || d.len() != 9 {
        return Err(VoteError::InvalidAccounts.into());
    }
    let option = d[0];
    let amount = read_u64(&d[1..]);
    validate_vote_input(option, amount)?;
    let (voter, proposal, receipt, source, vault, config, sp, tp) =
        (&a[0], &a[1], &a[2], &a[3], &a[4], &a[5], &a[6], &a[7]);
    if !voter.is_signer()
        || !voter.is_writable()
        || !proposal.is_writable()
        || !receipt.is_writable()
        || !source.is_writable()
        || !vault.is_writable()
        || sp.key() != &pinocchio_system::ID
        || tp.key() != &pinocchio_token::ID
    {
        return Err(VoteError::InvalidAccounts.into());
    }
    validate_config(config)?;
    let (pb, end, settled, pvault) = {
        let pd = proposal.try_borrow_data()?;
        if proposal.owner() != &ID
            || (pd.len() != LEGACY_PROPOSAL_LEN && pd.len() != PROPOSAL_LEN)
            || pd[0] != PROPOSAL_DISC
        {
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

fn validate_vote_input(option: u8, amount: u64) -> ProgramResult {
    if option > 1 {
        return Err(VoteError::InvalidOption.into());
    }
    if amount < MIN_VOTE_AMOUNT {
        return Err(VoteError::InvalidAmount.into());
    }
    Ok(())
}

fn settle(a: &[AccountInfo], d: &[u8]) -> ProgramResult {
    if a.len() != 6 || !d.is_empty() {
        return Err(VoteError::InvalidAccounts.into());
    }
    let (proposal, vault, config, reward_state, treasury, tp) =
        (&a[0], &a[1], &a[2], &a[3], &a[4], &a[5]);
    if !proposal.is_writable()
        || !vault.is_writable()
        || !config.is_writable()
        || !reward_state.is_writable()
        || !treasury.is_writable()
        || tp.key() != &pinocchio_token::ID
    {
        return Err(VoteError::InvalidAccounts.into());
    }
    let cb = validate_config(config)?;
    validate_reward_state(reward_state)?;
    let (bump, creator, nonce, end, settled, deposited, pvault) = {
        let pd = proposal.try_borrow_data()?;
        if proposal.owner() != &ID
            || (pd.len() != LEGACY_PROPOSAL_LEN && pd.len() != PROPOSAL_LEN)
            || pd[0] != PROPOSAL_DISC
        {
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
    record_recovery(config, reward_state, deposited)?;
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

fn initialize_rewards(a: &[AccountInfo], d: &[u8]) -> ProgramResult {
    if a.len() != 4 || !d.is_empty() {
        return Err(VoteError::InvalidAccounts.into());
    }
    let (payer, reward_state, config, sp) = (&a[0], &a[1], &a[2], &a[3]);
    if !payer.is_signer()
        || !payer.is_writable()
        || !reward_state.is_writable()
        || sp.key() != &pinocchio_system::ID
    {
        return Err(VoteError::InvalidAccounts.into());
    }
    validate_config(config)?;
    let (expected, bump) = find_program_address(&[b"rewards"], &ID);
    if reward_state.key() != &expected || reward_state.data_len() != 0 {
        return Err(VoteError::InvalidPda.into());
    }
    let (allocated, recovered) = {
        let cd = config.try_borrow_data()?;
        (read_u64(&cd[80..88]), read_u64(&cd[88..96]))
    };
    if allocated % ALLOCATION != 0 {
        return Err(VoteError::InvalidState.into());
    }
    let member_count = allocated / ALLOCATION;
    let bump_seed = [bump];
    let seeds = [Seed::from(b"rewards"), Seed::from(&bump_seed)];
    CreateAccount {
        from: payer,
        to: reward_state,
        lamports: rent_lamports(REWARD_STATE_LEN)?,
        space: REWARD_STATE_LEN as u64,
        owner: &ID,
    }
    .invoke_signed(&[Signer::from(&seeds)])?;
    let mut rd = reward_state.try_borrow_mut_data()?;
    rd[0] = REWARD_STATE_DISC;
    rd[1] = bump;
    rd[8..16].copy_from_slice(&member_count.to_le_bytes());
    let (reward_index, carry) = distribute_reward(0, 0, recovered, member_count)?;
    rd[16..24].copy_from_slice(&reward_index.to_le_bytes());
    rd[24..32].copy_from_slice(&carry.to_le_bytes());
    rd[32..40].copy_from_slice(&recovered.to_le_bytes());
    Ok(())
}

fn claim_reward(a: &[AccountInfo], d: &[u8]) -> ProgramResult {
    if a.len() != 9 || !d.is_empty() {
        return Err(VoteError::InvalidAccounts.into());
    }
    let (user, member, receipt, reward_state, config, treasury, dest, tp, sp) = (
        &a[0], &a[1], &a[2], &a[3], &a[4], &a[5], &a[6], &a[7], &a[8],
    );
    if !user.is_signer()
        || !user.is_writable()
        || !receipt.is_writable()
        || !reward_state.is_writable()
        || !treasury.is_writable()
        || !dest.is_writable()
        || tp.key() != &pinocchio_token::ID
        || sp.key() != &pinocchio_system::ID
    {
        return Err(VoteError::InvalidAccounts.into());
    }
    let cb = validate_config(config)?;
    validate_reward_state(reward_state)?;
    {
        let md = member.try_borrow_data()?;
        if member.owner() != &ID
            || md.len() != MEMBER_LEN
            || md[0] != MEMBER_DISC
            || user.key().as_ref() != &md[8..40]
        {
            return Err(VoteError::InvalidState.into());
        }
    }
    let (expected, bump) = find_program_address(&[b"reward", user.key()], &ID);
    if receipt.key() != &expected {
        return Err(VoteError::InvalidPda.into());
    }
    if receipt.data_len() == 0 {
        let bump_seed = [bump];
        let seeds = [
            Seed::from(b"reward"),
            Seed::from(user.key()),
            Seed::from(&bump_seed),
        ];
        CreateAccount {
            from: user,
            to: receipt,
            lamports: rent_lamports(REWARD_RECEIPT_LEN)?,
            space: REWARD_RECEIPT_LEN as u64,
            owner: &ID,
        }
        .invoke_signed(&[Signer::from(&seeds)])?;
        let mut receipt_data = receipt.try_borrow_mut_data()?;
        receipt_data[0] = REWARD_RECEIPT_DISC;
        receipt_data[1] = bump;
        receipt_data[8..40].copy_from_slice(user.key());
    }
    let reward_index = {
        let rd = reward_state.try_borrow_data()?;
        read_u64(&rd[16..24])
    };
    let (debt, claimed) = {
        let receipt_data = receipt.try_borrow_data()?;
        if receipt.owner() != &ID
            || receipt_data.len() != REWARD_RECEIPT_LEN
            || receipt_data[0] != REWARD_RECEIPT_DISC
            || user.key().as_ref() != &receipt_data[8..40]
        {
            return Err(VoteError::InvalidState.into());
        }
        (
            read_u64(&receipt_data[40..48]),
            read_u64(&receipt_data[48..56]),
        )
    };
    let amount = reward_index
        .checked_sub(debt)
        .ok_or(VoteError::MathOverflow)?;
    if amount == 0 {
        return Err(VoteError::InvalidAmount.into());
    }
    let cd = config.try_borrow_data()?;
    let mint = &cd[8..40];
    if treasury.key().as_ref() != &cd[40..72] {
        return Err(VoteError::InvalidMint.into());
    }
    let source = TokenAccount::from_account_info(treasury)?;
    let destination = TokenAccount::from_account_info(dest)?;
    if source.owner() != config.key()
        || source.mint() != mint
        || destination.owner() != user.key()
        || destination.mint() != mint
    {
        return Err(VoteError::InvalidMint.into());
    }
    drop(cd);
    let bump_seed = [cb];
    let seeds = [Seed::from(b"config"), Seed::from(&bump_seed)];
    Transfer {
        from: treasury,
        to: dest,
        authority: config,
        amount,
    }
    .invoke_signed(&[Signer::from(&seeds)])?;
    {
        let mut receipt_data = receipt.try_borrow_mut_data()?;
        receipt_data[40..48].copy_from_slice(&reward_index.to_le_bytes());
        let total = claimed.checked_add(amount).ok_or(VoteError::MathOverflow)?;
        receipt_data[48..56].copy_from_slice(&total.to_le_bytes());
    }
    let mut reward_data = reward_state.try_borrow_mut_data()?;
    let total_claimed = read_u64(&reward_data[40..48])
        .checked_add(amount)
        .ok_or(VoteError::MathOverflow)?;
    reward_data[40..48].copy_from_slice(&total_claimed.to_le_bytes());
    Ok(())
}

fn record_recovery(config: &AccountInfo, reward_state: &AccountInfo, amount: u64) -> ProgramResult {
    {
        let mut cd = config.try_borrow_mut_data()?;
        let recovered = read_u64(&cd[88..96])
            .checked_add(amount)
            .ok_or(VoteError::MathOverflow)?;
        cd[88..96].copy_from_slice(&recovered.to_le_bytes());
    }
    let mut rd = reward_state.try_borrow_mut_data()?;
    let members = read_u64(&rd[8..16]);
    let index = read_u64(&rd[16..24]);
    let carry = read_u64(&rd[24..32]);
    let total = read_u64(&rd[32..40])
        .checked_add(amount)
        .ok_or(VoteError::MathOverflow)?;
    let (next_index, next_carry) = distribute_reward(index, carry, amount, members)?;
    rd[16..24].copy_from_slice(&next_index.to_le_bytes());
    rd[24..32].copy_from_slice(&next_carry.to_le_bytes());
    rd[32..40].copy_from_slice(&total.to_le_bytes());
    Ok(())
}

fn distribute_reward(
    index: u64,
    carry: u64,
    amount: u64,
    members: u64,
) -> Result<(u64, u64), ProgramError> {
    let pool = carry.checked_add(amount).ok_or(VoteError::MathOverflow)?;
    if members == 0 {
        return Ok((index, pool));
    }
    let per_member = pool / members;
    let next_index = index
        .checked_add(per_member)
        .ok_or(VoteError::MathOverflow)?;
    Ok((next_index, pool % members))
}

fn validate_reward_state(reward_state: &AccountInfo) -> Result<u8, ProgramError> {
    if reward_state.owner() != &ID || reward_state.data_len() != REWARD_STATE_LEN {
        return Err(VoteError::InvalidState.into());
    }
    let data = reward_state.try_borrow_data()?;
    if data[0] != REWARD_STATE_DISC {
        return Err(VoteError::InvalidState.into());
    }
    let (expected, _) = find_program_address(&[b"rewards"], &ID);
    if reward_state.key() != &expected {
        return Err(VoteError::InvalidPda.into());
    }
    Ok(data[1])
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
        assert_eq!(PROPOSAL_LEN, 280);
        assert_eq!(PROPOSAL_FEE, 10_000_000);
        assert_eq!(MIN_VOTE_AMOUNT, 10_000);
        assert_eq!(REWARD_STATE_LEN, 64);
        assert_eq!(REWARD_RECEIPT_LEN, 56);
    }
    #[test]
    fn linear_math() {
        let vested = ALLOCATION as u128 * 15_768_000u128 / VESTING_SECONDS as u128;
        assert_eq!(vested, 500_000_000);
    }
    #[test]
    fn vote_amount_boundary() {
        assert!(validate_vote_input(0, MIN_VOTE_AMOUNT).is_ok());
        assert!(matches!(
            validate_vote_input(1, MIN_VOTE_AMOUNT - 1),
            Err(ProgramError::Custom(5))
        ));
        assert!(matches!(
            validate_vote_input(2, MIN_VOTE_AMOUNT),
            Err(ProgramError::Custom(11))
        ));
    }
    #[test]
    fn equal_reward_distribution_carries_dust() {
        let (index, carry) = distribute_reward(0, 0, 1_000_000_001, 100).unwrap();
        assert_eq!(index, 10_000_000);
        assert_eq!(carry, 1);
        let (index, carry) = distribute_reward(index, carry, 99, 100).unwrap();
        assert_eq!(index, 10_000_001);
        assert_eq!(carry, 0);
    }
    #[test]
    fn rewards_wait_when_there_are_no_members() {
        assert_eq!(distribute_reward(7, 3, 10, 0).unwrap(), (7, 13));
    }
}
