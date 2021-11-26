### Usage:
Calculate account balances with an example transactions file:
`cargo run --release examples/example_2.csv`

Run tests:
`cargo run --release test`

Code structure:
`lib.rs`: Business logic of transaction processing and account management.
`main.rs`: Handeles I/O, parsing and simply calls into the `lib.rs` library 
to perform the actual processing.

### Ambiguities & Assumptions
 - Should all transactions be disallowed for a frozen account?
   This implementation assumes that is the case.
 - Can disputes be for deposits?
   This implementation assumes disputes are allowed for both withdrawals and deposits.
   Further, this means the available funds for a client may be negative.
   When this happens, further withdrawals fail, but deposits continue to be processed.
   Also, chargebacks will not be processed.
 - Can we assume the values provided are "reasonable" (e.g. something like 100
   deposits of Price4::MAX doesn't happen). Is it okay to panic in this case?
   This implementation assumes the input can be malformed and carefully returns errors
   (no panicking) in case a transaction results in the overflow of any amount.
 - Should there be a way to reverse a account freeze?
   This implementation assumes there is no such method. Once an account is frozen it is never un-frozen.
 - Should we allow a transaction to be disputed multiple times?
   This implementation assumes that the transaction state is this:
             Withdraw/Deposit                  Dispute             Resolve/Chargeback
     Start -------------------->  Processed  ----------> InDispute -----------------> DisputeHandled
    So, in this implementation a transaction can only be disputed once.
 - This implementation assumes that the inputs are to be processed in a streaming-fashion. 
   i.e. we should not look ahead at future transactions to determine the outcome of the current
   transaction.

### Testing

Since this task has such a well-defined input and output, it was very convenient
to use snapshot testing with the `insta` crate. This makes it easy to update the
tests when the implementation changes, and makes it easy to add tests as well.

Right now the snapshot tests just test that the final output is as expected.
But, it would be more thorough to snapshot all intermediate states. This would 
also make the snapshots easier to understand since we can see the affect of each
transaction in the snapshot file.

We could also use property based testing (with quickcheck or similar tools) to test randomly
test out operations that try to break invariants in the code or get the code to panic.

The implementation uses strong types to avoid bugs with using the wrong variables
with the same type (e.g. ClientId, TransactionId are strongly typed).

### TODOs

- Determine the scale of the input and optimize as needed:
The currnet solution will likely not scale well for extremely large CSVs since all  
transactions are loaded into memory and never freed (since we need all transactions 
for disputes).

- Improve test-code organization: the tests should be part of the library, and not
of the binary.
One possibility here is to consider the parsing code part of the library itself.

- Parsing: Reject prices that have more than 4 decimals of precision.

- Testing:
    - add a test to ensure that price overflow does not panic
    - add a snapshot for each transaction in the test instead
      of just the end result. This would be much more robust
      and would reduce the number of tests needed as well.
    - add a large random stress test as a catch all for any 
      cases that were missed in other tests.
