/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use crate::objc::{id, objc_classes, ClassExports, HostObject};

pub struct SKPaymentTransactionHostObject {
    pub transaction_state: i64, // 0: Purchasing, 1: Purchased, 2: Failed, 3: Restored
    pub payment: id,
    pub error: id,
    pub transaction_receipt: id,
    pub transaction_identifier: id,
    pub transaction_date: id,
    pub original_transaction: id,
}

impl HostObject for SKPaymentTransactionHostObject {}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation SKPaymentTransaction: NSObject

- (i64)transactionState {
    env.objc.borrow::<SKPaymentTransactionHostObject>(this).transaction_state
}

- (id)payment {
    env.objc.borrow::<SKPaymentTransactionHostObject>(this).payment
}

- (id)error {
    env.objc.borrow::<SKPaymentTransactionHostObject>(this).error
}

- (id)transactionReceipt {
    env.objc.borrow::<SKPaymentTransactionHostObject>(this).transaction_receipt
}

- (id)transactionIdentifier {
    env.objc.borrow::<SKPaymentTransactionHostObject>(this).transaction_identifier
}

- (id)transactionDate {
    env.objc.borrow::<SKPaymentTransactionHostObject>(this).transaction_date
}

- (id)originalTransaction {
    env.objc.borrow::<SKPaymentTransactionHostObject>(this).original_transaction
}

@end

};
