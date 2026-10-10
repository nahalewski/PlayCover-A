/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Bind routing for the owned UIKit image: which app imports from UIKit,
//! QuartzCore and MetalANGLE resolve to the image instead of a cached or
//! bundled library. Pure lookup, no guest writes. The loader hook (agent A's
//! legacy/chained bind path) calls [Routes::route] for providers in
//! [Routes::owns_provider]. That hook is not wired yet.
use super::image::Layout;
use super::{INSTALL_NAME, METALANGLE, QUARTZCORE};
use std::collections::BTreeMap;

pub(in crate::a64) const OWNED_PROVIDERS: &[&str] = &[INSTALL_NAME, QUARTZCORE, METALANGLE];

pub(in crate::a64) struct Routes {
    map: BTreeMap<(String, String), u64>,
}

#[derive(Debug, Default)]
pub(in crate::a64) struct Coverage {
    pub routed: Vec<(String, String)>,
    pub missing_required: Vec<(String, String)>,
    pub missing_weak: Vec<(String, String)>,
}

impl Routes {
    pub(in crate::a64) fn from_layout(layout: &Layout) -> Self {
        Self {
            map: layout
                .exports()
                .into_iter()
                .map(|(provider, symbol, address)| ((provider, symbol), address))
                .collect(),
        }
    }
    pub(in crate::a64) fn owns_provider(provider: &str) -> bool {
        OWNED_PROVIDERS.contains(&provider)
    }
    /// The owned address for an import, or None if this layer does not
    /// implement it. A miss for an owned provider is a real gap (the loader
    /// must fail or weak-zero it, never fall back to the cached library).
    pub(in crate::a64) fn route(&self, provider: &str, symbol: &str) -> Option<u64> {
        self.map.get(&(provider.to_string(), symbol.to_string())).copied()
    }
    /// Classify (provider, symbol, weak) imports of owned providers.
    pub(in crate::a64) fn coverage<'a>(&self, imports: impl IntoIterator<Item = (&'a str, &'a str, bool)>) -> Coverage {
        let mut result = Coverage::default();
        for (provider, symbol, weak) in imports {
            if !Self::owns_provider(provider) {
                continue;
            }
            let key = (provider.to_string(), symbol.to_string());
            if self.map.contains_key(&key) {
                result.routed.push(key);
            } else if weak {
                result.missing_weak.push(key);
            } else {
                result.missing_required.push(key);
            }
        }
        result
    }
}
