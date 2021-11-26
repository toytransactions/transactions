use serde::{Deserialize, Serialize};
use std::{collections::HashMap, hash::Hash};
use thiserror::Error;

// TODO: We should use a type that guarantees _exactly_ 4 digits behind the decimal.
// `rust_decimal::Decimal` will accept arbitrary scale decimals -- these should be
// rejected when parsing.
pub type Price4 = rust_decimal::Decimal;

struct Funds {
    /// The funds available for withdrawing.
    available: Price4,
    /// The funds that are put on a temporary hold for disputed transactions.
    held: Price4,
}

impl Funds {
    pub fn new() -> Funds {
        Funds {
            available: Price4::ZERO,
            held: Price4::ZERO,
        }
    }

    pub fn total(&self) -> Price4 {
        self.available
            .checked_add(self.held)
            .expect("price overflow")
    }

    pub fn set(&mut self, available_funds: Price4, held_funds: Price4) -> Result<(), Error> {
        if available_funds.checked_add(held_funds).is_none() {
            return Err(Error::PriceOverflow(available_funds, held_funds));
        }
        self.available = available_funds;
        self.held = held_funds;
        Ok(())
    }
}

/// A client's latest account information.
pub struct Account {
    /// The funds in the account.
    funds: Funds,
    /// Whether or not the account is frozen.
    is_frozen: bool,
    /// The transactions made with this account.
    txs: HashMap<TransactionId, FundTransaction>,
}

impl Account {
    pub fn new() -> Account {
        Account {
            funds: Funds::new(),
            is_frozen: false,
            txs: HashMap::new(),
        }
    }

    pub fn available_funds(&self) -> Price4 {
        self.funds.available
    }

    pub fn held_funds(&self) -> Price4 {
        self.funds.held
    }

    pub fn total_funds(&self) -> Price4 {
        self.funds.total()
    }

    pub fn is_frozen(&self) -> bool {
        self.is_frozen
    }
}

/// A unique id assigned to each client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Deserialize, Serialize)]
pub struct ClientId(u16);

/// A globally-unique id assigned to each transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Deserialize, Serialize)]
pub struct TransactionId(u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Deposit,
    Withdrawal,
}

impl Side {
    fn opposite(&self) -> Side {
        match self {
            Side::Deposit => Side::Withdrawal,
            Side::Withdrawal => Side::Deposit,
        }
    }
}

fn calculate_amount(x: Price4, op: Side, y: Price4) -> Result<Price4, Error> {
    let res_opt = match op {
        Side::Deposit => x.checked_add(y),
        Side::Withdrawal => x.checked_sub(y),
    };
    res_opt.ok_or(Error::PriceOverflow(x, y))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionState {
    /// The transaction was successfully processed.
    Processed,
    /// The transaction is in dispute. This means the client has requested the
    /// transaction to be reversed.
    InDispute,
    /// The dispute was handled. This either means the transaction was reversed
    /// successfully, or the transaction was deemed to not need to be reversed.
    DisputeHandled,
}

/// A fund transaction represents either a deposit/withdraw.
struct FundTransaction {
    tx_id: TransactionId,
    amount: Price4,
    side: Side,
    state: TransactionState,
}

/// Processes transactions and manages client account information.
pub struct TransactionProcessor {
    accounts: HashMap<ClientId, Account>,
}

pub struct Deposit {
    pub client_id: ClientId,
    pub tx_id: TransactionId,
    pub amount: Price4,
}

pub struct Withdrawal {
    pub client_id: ClientId,
    pub tx_id: TransactionId,
    pub amount: Price4,
}

pub struct Dispute {
    pub client_id: ClientId,
    pub tx_id: TransactionId,
}

pub struct Resolve {
    pub client_id: ClientId,
    pub tx_id: TransactionId,
}

pub struct Chargeback {
    pub client_id: ClientId,
    pub tx_id: TransactionId,
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("invalid transaction id {0:?}")]
    InvalidTx(TransactionId),
    #[error("invalid transaction state (expected {expected:?}, found {actual:?})")]
    InvalidTxState {
        actual: TransactionState,
        expected: TransactionState,
    },
    #[error("invalid cliend id {0:?}")]
    InvalidClientId(ClientId),
    #[error("invalid price provided")]
    InvalidPrice,
    #[error("price overflow with {0:?} and {1:?}")]
    PriceOverflow(Price4, Price4),
    #[error("account is frozen")]
    AccountFrozen,
}

fn check_tx_state(actual: TransactionState, expected: TransactionState) -> Result<(), Error> {
    if actual != expected {
        return Err(Error::InvalidTxState { actual, expected });
    }
    return Ok(());
}

impl TransactionProcessor {
    pub fn new() -> TransactionProcessor {
        TransactionProcessor {
            accounts: HashMap::new(),
        }
    }

    /// Deposits `amount` value into `client_id`'s available balance as part of
    /// the transaction `tx_id`.
    /// Returns an error if:
    ///  - the transaction id is already used from another deposit/withdrawal
    ///  - the account is frozen
    ///  - `amount` is negative
    /// This function does not panic.
    pub fn process_deposit(&mut self, deposit: Deposit) -> Result<(), Error> {
        self.process_tx(
            deposit.client_id,
            FundTransaction {
                tx_id: deposit.tx_id,
                amount: deposit.amount,
                side: Side::Deposit,
                state: TransactionState::Processed,
            },
        )
    }

    /// Withdraws `amount` value from `client_id`'s available balance as part of
    /// the transaction `tx_id`.
    /// Returns an error if:
    ///  - the transaction id is already used from another deposit/withdrawal
    ///  - the available balance in the account is less than `amount`
    ///  - the account is frozen
    ///  - `amount` is negative
    /// This function does not panic.
    pub fn process_withdrawal(&mut self, withdrawal: Withdrawal) -> Result<(), Error> {
        self.process_tx(
            withdrawal.client_id,
            FundTransaction {
                tx_id: withdrawal.tx_id,
                amount: withdrawal.amount,
                side: Side::Withdrawal,
                state: TransactionState::Processed,
            },
        )
    }

    /// Marks the transaction `tx_id` for client `client_id` as being disputed.
    /// The funds associated with this transaction are removed from the available
    /// balance and kept in holding.
    /// Returns an error if:
    ///  - the transaction id `tx_id` doesn't exist for client `client_id`
    ///  - the transaction was already disputed / resolved / chargebacked.
    ///  - the account is frozen
    /// This function does not panic.
    pub fn process_dispute(&mut self, dispute: Dispute) -> Result<(), Error> {
        let (client_id, tx_id) = (dispute.client_id, dispute.tx_id);
        let account = self.get_account(client_id)?;
        let tx = account.txs.get_mut(&tx_id).ok_or(Error::InvalidTx(tx_id))?;
        check_tx_state(tx.state, TransactionState::Processed)?;

        // Held funds are increased, available funds are decreased.
        let opp_side = tx.side.opposite();
        let held_funds = calculate_amount(account.funds.held, tx.side, tx.amount)?;
        let available_funds = calculate_amount(account.funds.available, opp_side, tx.amount)?;
        account.funds.set(available_funds, held_funds)?;
        tx.state = TransactionState::InDispute;

        Ok(())
    }

    /// Marks the dispute for transaction `tx_id` for client `client_id` as resolved.
    /// The funds associated with this transaction are removed from holding and placed
    /// back into the client's available balance.
    /// Returns an error if:
    ///  - the transaction id `tx_id` doesn't exist for client `client_id`
    ///  - the transaction is not disputed
    ///  - the account is frozen
    /// This function does not panic.
    pub fn process_resolve(&mut self, resolve: Resolve) -> Result<(), Error> {
        let (client_id, tx_id) = (resolve.client_id, resolve.tx_id);
        let account = self.get_account(client_id)?;
        let tx = account.txs.get_mut(&tx_id).ok_or(Error::InvalidTx(tx_id))?;
        check_tx_state(tx.state, TransactionState::InDispute)?;

        // Held funds are decreased, available funds are increased.
        let opp_side = tx.side.opposite();
        let held_funds = calculate_amount(account.funds.held, opp_side, tx.amount)?;
        let available_funds = calculate_amount(account.funds.available, tx.side, tx.amount)?;
        account.funds.set(available_funds, held_funds)?;
        tx.state = TransactionState::DisputeHandled;

        Ok(())
    }

    /// Completes the dispute for transaction `tx_id` for client `client_id` by reversing
    /// the transaction. The funds are removed from holding and the account is marked frozen.
    /// Returns an error if:
    ///  - the transaction id `tx_id` doesn't exist for client `client_id`
    ///  - the transaction is not disputed
    ///  - the account is already frozen
    /// This function does not panic.
    pub fn process_chargeback(&mut self, chargeback: Chargeback) -> Result<(), Error> {
        let (client_id, tx_id) = (chargeback.client_id, chargeback.tx_id);
        let account = self.get_account(client_id)?;
        let tx = account.txs.get_mut(&tx_id).ok_or(Error::InvalidTx(tx_id))?;
        check_tx_state(tx.state, TransactionState::InDispute)?;

        // Held funds are decreased and account marked frozen.
        let opp_side = tx.side.opposite();
        let held_funds = calculate_amount(account.funds.held, opp_side, tx.amount)?;
        account.funds.set(account.funds.available, held_funds)?;
        account.is_frozen = true;
        tx.state = TransactionState::DisputeHandled;

        Ok(())
    }

    pub fn accounts(&self) -> &HashMap<ClientId, Account> {
        &self.accounts
    }

    fn process_tx(&mut self, client_id: ClientId, tx: FundTransaction) -> Result<(), Error> {
        if tx.amount < Price4::ZERO {
            return Err(Error::InvalidPrice);
        }

        let account = self.get_or_create_account(client_id)?;
        if account.txs.contains_key(&tx.tx_id) {
            return Err(Error::InvalidTx(tx.tx_id));
        }
        let available_funds = calculate_amount(account.funds.available, tx.side, tx.amount)?;
        // Disallow withdrawing if it results in negative available funds
        // This still allows depositing funds if there is a negative balance.
        if available_funds < Price4::ZERO && tx.side != Side::Deposit {
            return Err(Error::InvalidPrice);
        }
        account.funds.set(available_funds, account.funds.held)?;

        let old_tx = account.txs.insert(tx.tx_id, tx);
        assert!(old_tx.is_none());
        Ok(())
    }

    fn get_or_create_account(&mut self, client_id: ClientId) -> Result<&mut Account, Error> {
        let account = self.accounts.entry(client_id).or_insert_with(Account::new);
        if account.is_frozen {
            return Err(Error::AccountFrozen);
        }
        Ok(account)
    }

    fn get_account(&mut self, client_id: ClientId) -> Result<&mut Account, Error> {
        let account = self
            .accounts
            .get_mut(&client_id)
            .ok_or(Error::InvalidClientId(client_id))?;
        if account.is_frozen {
            return Err(Error::AccountFrozen);
        }
        Ok(account)
    }
}
