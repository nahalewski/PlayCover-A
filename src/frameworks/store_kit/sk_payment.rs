/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use crate::objc::{id, msg, nil, objc_classes, ClassExports, HostObject};

pub struct SKPaymentHostObject {
    pub product_identifier: id,
    pub quantity: i64,
    pub request_data: id,
}

impl HostObject for SKPaymentHostObject {}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation SKPayment: NSObject

+ (id)paymentWithProduct:(id)product {
    let pid: id = msg![env; product productIdentifier];
    let host_obj = Box::new(SKPaymentHostObject {
        product_identifier: pid,
        quantity: 1,
        request_data: nil,
    });
    env.objc.alloc_object(this, host_obj, &mut env.mem)
}

+ (id)paymentWithProductIdentifier:(id)identifier {
    let host_obj = Box::new(SKPaymentHostObject {
        product_identifier: identifier,
        quantity: 1,
        request_data: nil,
    });
    env.objc.alloc_object(this, host_obj, &mut env.mem)
}

- (id)productIdentifier {
    env.objc.borrow::<SKPaymentHostObject>(this).product_identifier
}

- (i64)quantity {
    env.objc.borrow::<SKPaymentHostObject>(this).quantity
}

- (id)requestData {
    env.objc.borrow::<SKPaymentHostObject>(this).request_data
}

@end

@implementation SKMutablePayment: SKPayment

+ (id)paymentWithProduct:(id)product {
    let pid: id = msg![env; product productIdentifier];
    let host_obj = Box::new(SKPaymentHostObject {
        product_identifier: pid,
        quantity: 1,
        request_data: nil,
    });
    env.objc.alloc_object(this, host_obj, &mut env.mem)
}

+ (id)paymentWithProductIdentifier:(id)identifier {
    let host_obj = Box::new(SKPaymentHostObject {
        product_identifier: identifier,
        quantity: 1,
        request_data: nil,
    });
    env.objc.alloc_object(this, host_obj, &mut env.mem)
}

- (())setProductIdentifier:(id)identifier {
    env.objc.borrow_mut::<SKPaymentHostObject>(this).product_identifier = identifier;
}

- (())setQuantity:(i64)quantity {
    env.objc.borrow_mut::<SKPaymentHostObject>(this).quantity = quantity;
}

- (())setRequestData:(id)data {
    env.objc.borrow_mut::<SKPaymentHostObject>(this).request_data = data;
}

@end

};
