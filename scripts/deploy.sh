#!/usr/bin/env bash
set -euo pipefail
RPC="https://api.devnet.solana.com"
PROGRAM_KEYPAIR="${1:-vote-program-keypair.json}"
NO_DNA=1 cargo build-sbf
solana program deploy target/deploy/civic_vote_program.so --program-id "$PROGRAM_KEYPAIR" --url "$RPC"

