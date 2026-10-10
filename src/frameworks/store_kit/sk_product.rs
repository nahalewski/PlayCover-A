/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use crate::objc::{id, msg, msg_class, nil, objc_classes, SEL, ClassExports, HostObject};

pub struct SKProductHostObject {
    pub product_identifier: id,
    pub localized_title: id,
    pub localized_description: id,
}

impl HostObject for SKProductHostObject {}

pub struct SKProductsRequestHostObject {
    pub product_identifiers: id, // NSSet
    pub delegate: id,
}

impl HostObject for SKProductsRequestHostObject {}

pub struct SKProductsResponseHostObject {
    pub products: id, // NSArray
    pub invalid_product_identifiers: id, // NSArray
}

impl HostObject for SKProductsResponseHostObject {}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation SKProduct: NSObject

- (id)productIdentifier {
    env.objc.borrow::<SKProductHostObject>(this).product_identifier
}

- (id)localizedTitle {
    env.objc.borrow::<SKProductHostObject>(this).localized_title
}

- (id)localizedDescription {
    env.objc.borrow::<SKProductHostObject>(this).localized_description
}

- (id)price {
    // When unlock_store_purchases is active (or by default for offline play),
    // items are free (0.0).
    msg_class![env; NSNumber numberWithInt:0]
}

- (id)priceLocale {
    msg_class![env; NSLocale currentLocale]
}

@end

@implementation SKProductsResponse: NSObject

- (id)products {
    env.objc.borrow::<SKProductsResponseHostObject>(this).products
}

- (id)invalidProductIdentifiers {
    env.objc.borrow::<SKProductsResponseHostObject>(this).invalid_product_identifiers
}

@end

@implementation SKProductsRequest: NSObject

- (id)initWithProductIdentifiers:(id)ids {
    let host_obj = Box::new(SKProductsRequestHostObject {
        product_identifiers: ids,
        delegate: nil,
    });
    env.objc.alloc_object(this, host_obj, &mut env.mem)
}

- (())setDelegate:(id)delegate {
    env.objc.borrow_mut::<SKProductsRequestHostObject>(this).delegate = delegate;
}

- (id)delegate {
    env.objc.borrow::<SKProductsRequestHostObject>(this).delegate
}

- (())start {
    let (set, delegate) = {
        let h = env.objc.borrow::<SKProductsRequestHostObject>(this);
        (h.product_identifiers, h.delegate)
    };

    // Extract product IDs from NSSet or array if present
    let mut product_objs: Vec<id> = Vec::new();
    if set != nil {
        let all_objects: id = msg![env; set allObjects];
        let count: u64 = msg![env; all_objects count];
        let sk_product_class = env.objc.get_known_class("SKProduct", &mut env.mem);
        for i in 0..count {
            let pid: id = msg![env; all_objects objectAtIndex:i];
            let host_obj = Box::new(SKProductHostObject {
                product_identifier: pid,
                localized_title: pid,
                localized_description: pid,
            });
            let prod_obj = env.objc.alloc_object(sk_product_class, host_obj, &mut env.mem);
            product_objs.push(prod_obj);
        }
    }

    let products_arr: id = crate::frameworks::foundation::ns_array::from_vec(env, product_objs);
    let invalid_arr: id = crate::frameworks::foundation::ns_array::from_vec(env, Vec::new());

    let sk_response_class = env.objc.get_known_class("SKProductsResponse", &mut env.mem);
    let resp_host = Box::new(SKProductsResponseHostObject {
        products: products_arr,
        invalid_product_identifiers: invalid_arr,
    });
    let response_obj = env.objc.alloc_object(sk_response_class, resp_host, &mut env.mem);

    if delegate != nil {
        let sel: SEL = env.objc.register_host_selector("productsRequest:didReceiveResponse:".to_string(), &mut env.mem);
        let responds: bool = msg![env; delegate respondsToSelector:sel];
        if responds {
            let _: () = msg![env; delegate productsRequest:this didReceiveResponse:response_obj];
        }

        let sel_finish: SEL = env.objc.register_host_selector("requestDidFinish:".to_string(), &mut env.mem);
        let responds_finish: bool = msg![env; delegate respondsToSelector:sel_finish];
        if responds_finish {
            let _: () = msg![env; delegate requestDidFinish:this];
        }
    }
}

- (())cancel {
    // No-op
}

@end

@implementation SKReceiptRefreshRequest: NSObject

- (id)initWithReceiptProperties:(id)_props {
    let host_obj = Box::new(SKProductsRequestHostObject {
        product_identifiers: nil,
        delegate: nil,
    });
    env.objc.alloc_object(this, host_obj, &mut env.mem)
}

- (())setDelegate:(id)delegate {
    env.objc.borrow_mut::<SKProductsRequestHostObject>(this).delegate = delegate;
}

- (id)delegate {
    env.objc.borrow::<SKProductsRequestHostObject>(this).delegate
}

- (())start {
    let delegate = env.objc.borrow::<SKProductsRequestHostObject>(this).delegate;
    if delegate != nil {
        let sel_finish: SEL = env.objc.register_host_selector("requestDidFinish:".to_string(), &mut env.mem);
        let responds_finish: bool = msg![env; delegate respondsToSelector:sel_finish];
        if responds_finish {
            let _: () = msg![env; delegate requestDidFinish:this];
        }
    }
}

@end

};
