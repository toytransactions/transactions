use serde::{Deserialize, Serialize};
use transactions::{Chargeback, Deposit, Dispute, Resolve, Withdrawal};
use transactions::{ClientId, Error, Price4, TransactionId, TransactionProcessor};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TransactionInfoKind {
    Deposit,
    Withdrawal,
    Dispute,
    Resolve,
    Chargeback,
}

#[derive(Debug, Deserialize)]
struct TransactionInfo {
    #[serde(rename = "type")]
    kind: TransactionInfoKind,
    #[serde(rename = "client")]
    client_id: ClientId,
    #[serde(rename = "tx")]
    tx_id: TransactionId,
    amount: Option<Price4>,
}

#[derive(Debug, Deserialize, Serialize)]
struct AccountInfo {
    #[serde(rename = "client")]
    client_id: ClientId,
    #[serde(rename = "available")]
    available_funds: Price4,
    #[serde(rename = "held")]
    held_funds: Price4,
    #[serde(rename = "total")]
    total_funds: Price4,
    #[serde(rename = "locked")]
    is_frozen: bool,
}

fn process(
    transaction_processor: &mut TransactionProcessor,
    tx_info: &TransactionInfo,
) -> Result<(), Error> {
    match tx_info.kind {
        TransactionInfoKind::Deposit => transaction_processor.process_deposit(Deposit {
            client_id: tx_info.client_id,
            tx_id: tx_info.tx_id,
            // TODO: Use separate error type and not a internal library error type.
            amount: tx_info.amount.ok_or(Error::InvalidPrice)?,
        }),
        TransactionInfoKind::Withdrawal => transaction_processor.process_withdrawal(Withdrawal {
            client_id: tx_info.client_id,
            tx_id: tx_info.tx_id,
            amount: tx_info.amount.ok_or(Error::InvalidPrice)?,
        }),
        TransactionInfoKind::Dispute => transaction_processor.process_dispute(Dispute {
            client_id: tx_info.client_id,
            tx_id: tx_info.tx_id,
        }),
        TransactionInfoKind::Resolve => transaction_processor.process_resolve(Resolve {
            client_id: tx_info.client_id,
            tx_id: tx_info.tx_id,
        }),
        TransactionInfoKind::Chargeback => transaction_processor.process_chargeback(Chargeback {
            client_id: tx_info.client_id,
            tx_id: tx_info.tx_id,
        }),
    }
}

fn run<R, W, E>(instream: R, outstream: W, mut errstream: E)
where
    R: std::io::Read,
    W: std::io::Write,
    E: std::io::Write,
{
    // 1) Parse transactions from `instream` and process them.
    let mut transaction_processor = TransactionProcessor::new();
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .delimiter(b',')
        .from_reader(instream);
    for result in reader.deserialize() {
        let tx_info: TransactionInfo = match result {
            Ok(tx_info) => tx_info,
            Err(e) => {
                writeln!(errstream, "deserialize failed: {}", e).expect("write failed");
                continue;
            }
        };
        if let Err(e) = process(&mut transaction_processor, &tx_info) {
            writeln!(errstream, "failed to process `{:?}`: {}", tx_info, e).expect("write failed");
        }
    }

    // 2) Get all client account infos.
    let mut account_infos = Vec::new();
    for (client_id, account) in transaction_processor.accounts().iter() {
        account_infos.push(AccountInfo {
            client_id: *client_id,
            available_funds: account.available_funds(),
            held_funds: account.held_funds(),
            total_funds: account.total_funds(),
            is_frozen: account.is_frozen(),
        });
    }
    // Sort the account infos by client id so the output is deterministic.
    account_infos.sort_by_key(|account| account.client_id);

    // 3) Print the account infos to outstream in csv format.
    let mut writer = csv::Writer::from_writer(outstream);
    for account_info in account_infos.iter() {
        if let Err(e) = writer.serialize(account_info) {
            writeln!(errstream, "serialize failed: {}", e).expect("write failed");
        }
    }
    writer.flush().expect("write failed");
    errstream.flush().expect("write failed");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let filepath = args
        .get(1)
        .expect("Usage: ./transactions <csv filepath with transactions>");
    let file = std::fs::File::open(filepath).expect("could not open csv file");
    run(file, std::io::stdout(), std::io::stderr());
}

#[cfg(test)]
mod test {
    use super::*;
    use std::io::BufWriter;

    fn run_snapshot_test(input: &str) {
        let mut outstream = BufWriter::new(Vec::new());
        let mut errstream = BufWriter::new(Vec::new());
        run(input.as_bytes(), &mut outstream, &mut errstream);
        let outstring = String::from_utf8(outstream.into_inner().unwrap()).unwrap();
        let errstring = String::from_utf8(errstream.into_inner().unwrap()).unwrap();
        let all_output = format!("{}Stderr:\n{}", outstring, errstring);
        insta::assert_snapshot!(all_output);
    }

    #[test]
    fn test_serde() {
        // Tests that transaction type, integers, optional prices, booleans are correctly
        // serialized and deserialized.
        let input = "
            type,       client, tx, amount
            deposit,    1, 3, 1
            deposit,    1, 5, .5
            deposit,    1, 6, 0.2
            withdrawal, 1, 4, .0001
            dispute,    1, 5,
            resolve,    1, 5,
            dispute,    1, 4,
            dispute,    1, 6,
            chargeback, 1, 4,
            deposit,    2, 15, 100.03
            dispute,    2, 15,
            chargeback, 2, 15,";
        run_snapshot_test(input);
    }

    #[test]
    fn test_unknown_transaction_id() {
        // Tests that disputes, resolves, and chargebacks for unknown clients / transactions
        // are ignored.
        let input = "
            type,       client, tx, amount
            deposit,    1, 5, 1.5
            dispute,    1, 6,
            chargeback, 1, 6,
            resolve,    1, 6,
            deposit,    1, 6, 2.0
            dispute,    1, 6,
            dispute,    2, 5,
            chargeback, 2, 5,
            resolve,    2, 5,";
        run_snapshot_test(input);
    }

    #[test]
    fn test_duplicate_tx_id_rejected() {
        // Tests that duplicate transaction ids for a client are rejected
        let input = "
            type, client, tx, amount
            deposit,    1, 1, 1.0
            withdrawal, 1, 1, 0.5
            deposit,    1, 1, 2.0";
        run_snapshot_test(input);
    }

    #[test]
    fn test_deposit_withdrawal() {
        // Tests withdrawing tests the exact available balance works and withdrawing
        // more than available fails.
        let input = "
            type,       client, tx, amount
            deposit,    1, 100, 1.3
            deposit,    1, 1, 0.2
            withdrawal, 1, 2, .0001
            withdrawal, 1, 3, 1.4999
            withdrawal, 1, 4, .0001
            deposit,    1, 5, 2.0
            withdrawal, 1, 6, 2.0001";
        run_snapshot_test(input);
    }

    #[test]
    fn test_dispute() {
        // Tests that disputes result in balance being held which
        // cannot be used for withdrawing.
        let input = "
            type,       client, tx, amount
            deposit,    1, 5, 1.0
            deposit,    1, 6, 2
            dispute,    1, 5,
            withdrawal, 1, 7, 2.5
            withdrawal, 1, 8, 1.5";
        run_snapshot_test(input);
    }

    #[test]
    fn test_negative_available_on_dispute() {
        // Tests that disputes can result in negative available
        let input = "
            type,       client, tx, amount
            deposit,    1, 5, 10
            deposit,    1, 6, 20
            withdrawal, 1, 7, 25
            dispute,    1, 6,";
        run_snapshot_test(input);
    }

    #[test]
    fn test_negative_held_on_dispute() {
        // Tests that disputes can result in negative held
        let input = "
            type,       client, tx, amount
            deposit,    1, 5, 10
            withdrawal, 1, 6, 5
            dispute,    1, 6,";
        run_snapshot_test(input);
    }

    #[test]
    fn test_cannot_withdraw_on_negative_balance() {
        // Tests that withdrawing when balance is negative fails, but depositing still works
        let input = "
            type,       client, tx, amount
            deposit,    1, 5, 10
            deposit,    1, 6, 20
            withdrawal, 1, 7, 25
            dispute,    1, 6,
            withdrawal, 1, 8, 4
            deposit,    1, 9, 5
            withdrawal, 1,10, 3
            deposit,    1,11, 25
            withdrawal, 1,12, 5";
        run_snapshot_test(input);
    }

    #[test]
    fn test_resolve() {
        // Tests that resolves result in held money put back in available.
        let input = "
            type,       client, tx, amount
            deposit,    1, 5, 2.0
            dispute,    1, 5,
            withdrawal, 1, 6, 1.0
            resolve,    1, 5,
            withdrawal, 1, 7, 0.5";
        run_snapshot_test(input);
    }

    #[test]
    fn test_multiple_dispute_disallowed() {
        // Tests that multiple disputes are disallowed for a transaction,
        // even if the transaction was resolved.
        let input = "
            type,       client, tx, amount
            deposit,    1, 5, 2.0
            dispute,    1, 5,
            dispute,    1, 5,
            resolve,    1, 5,
            dispute,    1, 5,
            withdrawal, 1, 6, 0.5";
        run_snapshot_test(input);
    }

    #[test]
    fn test_chargeback() {
        // Tests that chargebacks result in frozen accounts
        // where no more transactions are allowed.
        let input = "
            type,       client, tx, amount
            deposit,    1, 4, 1.0
            deposit,    1, 5, 2.0
            dispute,    1, 5,
            chargeback, 1, 5,
            withdrawal, 1, 6, 0.5
            deposit,    1, 7, 0.1
            dispute,    1, 7,
            resolve,    1, 7,
            deposit,    2, 8, 1.0";
        run_snapshot_test(input);
    }

    #[test]
    fn test_negative_available_on_chargeback() {
        // Tests that chargebacks can result in negative balances.
        let input = "
            type,       client, tx, amount
            deposit,    1, 3, 0.7
            deposit,    1, 4, 0.3
            deposit,    1, 5, 2.0
            withdrawal, 1, 6, 2.5
            dispute,    1, 4,
            resolve,    1, 4,
            withdrawal, 1, 7, 0.1
            dispute,    1, 3,
            dispute,    1, 5,
            chargeback, 1, 5,";
        run_snapshot_test(input);
    }

    #[test]
    fn test_multiple_clients() {
        // Tests using multiple clients
        let input = "
            type,       client, tx, amount
            withdrawal, 2, 1, 10
            deposit,    1, 2, 100
            deposit,    1,10, 50
            withdrawal, 2, 3, 10
            deposit,    2, 4, 200
            withdrawal, 2, 5, 10
            dispute,    1, 5,
            resolve,    1, 5,
            deposit,    3, 6, 75
            deposit,    3, 7, 10
            withdrawal, 3, 8, 80
            dispute,    2, 6,
            dispute,    3, 6,
            chargeback, 3, 6,
            dispute,    1,10,";
        run_snapshot_test(input);
    }
}
