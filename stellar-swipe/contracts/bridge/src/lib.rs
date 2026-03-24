#![no_std]

pub mod monitoring;
pub mod governance;

pub use monitoring::{
    ChainFinalityConfig, ChainId, MonitoredTransaction, MonitoringStatus, VerificationMethod,
    BridgeTransfer, TransferStatus,
    monitor_source_transaction, get_monitored_tx, check_for_reorg, handle_reorg,
    update_transaction_confirmation_count, mark_transaction_failed, create_bridge_transfer,
    add_validator_signature, approve_transfer_for_minting, complete_transfer,
    get_chain_finality_config, set_chain_finality_config,
};

pub use governance::{
    BridgeGovernance, GovernanceProposal, ProposalType, ProposalStatus,
    BridgeSecurityConfig, Bridge, BridgeStatus,
    initialize_bridge_governance, initialize_bridge,
    create_bridge_proposal, sign_bridge_proposal, execute_bridge_proposal,
    emergency_execute_proposal, cancel_proposal,
    get_proposal_details, get_bridge_proposals, get_pending_proposals,
    rotate_bridge_signers, add_signer, remove_signer,
    get_bridge_status, get_bridge_validators, get_governance_signers,
    get_required_signatures, is_signer, is_validator,
};
