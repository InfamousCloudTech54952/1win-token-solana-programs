//! Research-only LiteSVM tests of the documented conservative Sale reserve.
//! Uses dummy local accounts and program binaries; never connects to real Solana.
//! Upstream source baseline: 68c301dfb8a2d4d0522bcd5a1d471a7905055fdc.

use sale_test_common::{
    activate_claim_ix, claim_ix, init_aggregate_ix, initialize_ix, proceeds_vault_pda,
    purchase_ix, refund_vault_pda, token_vault_pda, wallet_token_ata, wallet_usdc_ata,
    TestEnv,
};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

#[allow(clippy::result_large_err)]
fn send(
    env: &mut TestEnv,
    ix: solana_instruction::Instruction,
    signer: &Keypair,
) -> litesvm::types::TransactionResult {
    env.send_ixs(&[ix], &signer.pubkey(), &[signer])
}

struct Outcome {
    participants: u64,
    proceeds: u64,
    refund_vault_initial: u64,
    refund_vault_final: u64,
    paid_tokens: [u64; 2],
    paid_refunds: [u64; 2],
}

fn scenario(empty_wallets: usize, emission: u64, contribution: u64) -> Outcome {
    let mut env = TestEnv::new();
    let (usdc, win) = (env.usdc_mint, env.win_mint);
    let owner = env.actors.owner.insecure_clone();
    let admin = env.actors.admin.insecure_clone();
    let buyers = [
        env.actors.users[0].insecure_clone(),
        env.actors.users[1].insecure_clone(),
    ];
    let mut p = env.valid_init_params();
    p.price_usdc = 30_000;
    p.emission = emission;
    if contribution < p.min_per_tx {
        p.min_per_tx = 1;
        p.max_per_tx = 100;
    }
    send(
        &mut env,
        initialize_ix(owner.pubkey(), usdc, win, &p),
        &owner,
    )
    .expect("initialize");

    env.warp_to(p.sale_start_date);
    for w in &buyers {
        let ata = env.create_ata(&w.pubkey(), &usdc);
        env.mint_to(&usdc, &ata, contribution);
        env.expire_blockhash();
        send(&mut env, init_aggregate_ix(w.pubkey()), w)
            .expect("init funded buyer aggregate");
        env.expire_blockhash();
        send(&mut env, purchase_ix(w.pubkey(), ata, 0, contribution), w)
            .expect("purchase");
    }

    // A different signer pays the rent for each independent zero-net aggregate.
    for _ in 0..empty_wallets {
        let w = env.new_user();
        env.expire_blockhash();
        send(&mut env, init_aggregate_ix(w.pubkey()), &w)
            .expect("init empty aggregate");
    }
    let participants = env.sale_state().participants;
    assert_eq!(participants, 2 + empty_wallets as u64);
    assert_eq!(env.sale_state().deposits_usdc_contributed, 2 * contribution);

    env.warp_to(p.sale_end_date);
    env.mint_to(&win, &token_vault_pda(), emission);
    env.expire_blockhash();
    send(
        &mut env,
        activate_claim_ix(admin.pubkey(), usdc, Some(refund_vault_pda())),
        &admin,
    )
    .expect("activate_claim");

    let proceeds = env.token_balance(&proceeds_vault_pda());
    let refund_vault_initial = env.token_balance(&refund_vault_pda());
    assert_eq!(proceeds + refund_vault_initial, 2 * contribution);

    env.warp_to(p.claim_start_date);
    let mut paid_tokens = [0; 2];
    let mut paid_refunds = [0; 2];
    for (i, w) in buyers.iter().enumerate() {
        let usdc_before = env.token_balance(&wallet_usdc_ata(&w.pubkey(), &usdc));
        env.expire_blockhash();
        send(&mut env, claim_ix(w.pubkey(), usdc, win, true), w)
            .expect("claim");
        paid_tokens[i] = env.token_balance(&wallet_token_ata(&w.pubkey(), &win));
        paid_refunds[i] =
            env.token_balance(&wallet_usdc_ata(&w.pubkey(), &usdc)) - usdc_before;
    }
    let refund_vault_final = env.token_balance(&refund_vault_pda());
    assert_eq!(
        refund_vault_initial,
        paid_refunds.iter().sum::<u64>() + refund_vault_final,
        "refund-vault conservation"
    );
    Outcome {
        participants,
        proceeds,
        refund_vault_initial,
        refund_vault_final,
        paid_tokens,
        paid_refunds,
    }
}

#[test]
fn empty_accounts_change_reserve_not_buyer_claims() {
    let base = scenario(0, 100_000_000_000, 20_000_000);
    let extra = scenario(13, 100_000_000_000, 20_000_000);

    assert_eq!(base.participants, 2);
    assert_eq!(extra.participants, 15);
    assert_eq!(base.proceeds, 30_000_000 - 2);
    assert_eq!(extra.proceeds, 30_000_000 - 15);
    assert_eq!(base.refund_vault_initial, 10_000_000 + 2);
    assert_eq!(extra.refund_vault_initial, 10_000_000 + 15);
    assert_eq!(base.paid_tokens, [50_000_000_000; 2]);
    assert_eq!(base.paid_refunds, [5_000_000; 2]);
    assert_eq!(extra.paid_tokens, base.paid_tokens);
    assert_eq!(extra.paid_refunds, base.paid_refunds);
    assert_eq!(extra.refund_vault_final, 15);
}

#[test]
fn tiny_soft_cap_boundary_defers_proceeds_but_keeps_refunds_solvent() {
    // floor(10_000 * 30_000 / 100_000_000) == 3 micro USDC.
    // This deliberately tiny configuration is for boundary testing only.
    let base = scenario(0, 10_000, 20);
    let extra = scenario(3, 10_000, 20);

    assert_eq!(base.proceeds, 1);
    assert_eq!(extra.proceeds, 0);
    assert_eq!(extra.refund_vault_initial, 40);
    assert_eq!(base.paid_tokens, [5_000; 2]);
    assert_eq!(base.paid_refunds, [18; 2]);
    assert_eq!(extra.paid_tokens, base.paid_tokens);
    assert_eq!(extra.paid_refunds, base.paid_refunds);
    assert_eq!(extra.refund_vault_final, 4);
}
