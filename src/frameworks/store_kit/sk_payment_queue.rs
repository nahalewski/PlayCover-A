/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use crate::frameworks::foundation::ns_string::from_rust_string;
use crate::frameworks::store_kit::sk_transaction::SKPaymentTransactionHostObject;
use crate::objc::{id, msg, msg_class, nil, objc_classes, SEL, ClassExports, HostObject};
use std::sync::atomic::{AtomicU64, Ordering};

static TX_COUNTER: AtomicU64 = AtomicU64::new(1001);

pub struct SKPaymentQueueHostObject {
    pub observers: Vec<id>,
    pub transactions: Vec<id>,
}

impl HostObject for SKPaymentQueueHostObject {}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation SKPaymentQueue: NSObject

+ (id)defaultQueue {
    let queue_class = env.objc.get_known_class("SKPaymentQueue", &mut env.mem);
    let host_obj = Box::new(SKPaymentQueueHostObject {
        observers: Vec::new(),
        transactions: Vec::new(),
    });
    env.objc.alloc_object(queue_class, host_obj, &mut env.mem)
}

+ (bool)canMakePayments {
    true
}

- (())addTransactionObserver:(id)observer {
    if observer == nil {
        return;
    }
    let observers = &mut env.objc.borrow_mut::<SKPaymentQueueHostObject>(this).observers;
    if !observers.contains(&observer) {
        observers.push(observer);
    }
}

- (())removeTransactionObserver:(id)observer {
    if observer == nil {
        return;
    }
    let observers = &mut env.objc.borrow_mut::<SKPaymentQueueHostObject>(this).observers;
    observers.retain(|&o| o != observer);
}

- (id)transactions {
    let txs = env.objc.borrow::<SKPaymentQueueHostObject>(this).transactions.clone();
    crate::frameworks::foundation::ns_array::from_vec(env, txs)
}

- (())addPayment:(id)payment {
    log!("StoreKit: addPayment called (unlock_store_purchases={})", env.options.unlock_store_purchases);
    
    let tx_num = TX_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tx_id_str = from_rust_string(env, format!("tx_{}", tx_num));
    let receipt_str = from_rust_string(env, "STOREKIT_AUTO_GRANT_RECEIPT".to_string());
    let tx_receipt: id = msg![env; receipt_str dataUsingEncoding:4u32];
    let tx_date = msg_class![env; NSDate date];

    let tx_class = env.objc.get_known_class("SKPaymentTransaction", &mut env.mem);
    let tx_host = Box::new(SKPaymentTransactionHostObject {
        transaction_state: 1, // Purchased
        payment,
        error: nil,
        transaction_receipt: tx_receipt,
        transaction_identifier: tx_id_str,
        transaction_date: tx_date,
        original_transaction: nil,
    });
    let tx_obj = env.objc.alloc_object(tx_class, tx_host, &mut env.mem);

    env.objc.borrow_mut::<SKPaymentQueueHostObject>(this).transactions.push(tx_obj);

    let tx_array: id = crate::frameworks::foundation::ns_array::from_vec(env, vec![tx_obj]);
    let observers = env.objc.borrow::<SKPaymentQueueHostObject>(this).observers.clone();

    let sel: SEL = env.objc.register_host_selector("paymentQueue:updatedTransactions:".to_string(), &mut env.mem);
    for observer in observers {
        let responds: bool = msg![env; observer respondsToSelector:sel];
        if responds {
            let _: () = msg![env; observer paymentQueue:this updatedTransactions:tx_array];
        }
    }
}

- (())restoreCompletedTransactions {
    log!("StoreKit: restoreCompletedTransactions called");
    let observers = env.objc.borrow::<SKPaymentQueueHostObject>(this).observers.clone();
    let sel: SEL = env.objc.register_host_selector("paymentQueueRestoreCompletedTransactionsFinished:".to_string(), &mut env.mem);
    for observer in observers {
        let responds: bool = msg![env; observer respondsToSelector:sel];
        if responds {
            let _: () = msg![env; observer paymentQueueRestoreCompletedTransactionsFinished:this];
        }
    }
}

- (())finishTransaction:(id)transaction {
    log!("StoreKit: finishTransaction called");
    let transactions = &mut env.objc.borrow_mut::<SKPaymentQueueHostObject>(this).transactions;
    transactions.retain(|&t| t != transaction);
}

@end

};
