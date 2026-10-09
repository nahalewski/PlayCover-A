/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Blocks advertising networks (and advertising attribution trackers) for
//! every emulated app.
//!
//! The games touchHLE runs are old; their ad SDKs (MoPub, Unity Ads, AdMob,
//! ...) only serve ads that earn someone money and track the player. Name
//! lookups and HTTP requests to these hosts fail like an unreachable server
//! would, so the SDKs report "no ad available" and the game carries on.
//! `--allow-ads` turns the block off (for debugging an ad SDK).

use std::sync::atomic::{AtomicBool, Ordering};

static ALLOW_ADS: AtomicBool = AtomicBool::new(false);

pub fn set_allow_ads(allow: bool) {
    ALLOW_ADS.store(allow, Ordering::Relaxed);
}

/// Domains whose subdomains are blocked too.
const BLOCKED_DOMAINS: &[&str] = &[
    // Google
    "doubleclick.net",
    "googlesyndication.com",
    "googleadservices.com",
    "admob.com",
    "googletagservices.com",
    "adservice.google.com",
    // MoPub / Twitter
    "mopub.com",
    "mopubi.com",
    // Unity Ads / Applifier
    "unityads.unity3d.com",
    "applifier.com",
    "unityads.unity3d.com",
    "cdn-store-icons.uca.cloud.unity3d.com",
    "adserver.unityads.unity3d.com",
    // Other ad networks
    "adcolony.com",
    "vungle.com",
    "chartboost.com",
    "tapjoy.com",
    "inmobi.com",
    "millennialmedia.com",
    "jumptap.com",
    "flurry.com",
    "applovin.com",
    "ironsrc.com",
    "supersonicads.com",
    "startapp.com",
    "smaato.net",
    "adnxs.com",
    "adsrvr.org",
    "amazon-adsystem.com",
    "heyzap.com",
    "fyber.com",
    "playhaven.com",
    "revmob.com",
    "leadbolt.net",
    "airpush.com",
    "mdotm.com",
    "greystripe.com",
    "adwhirl.com",
    "mobclix.com",
    "quattrowireless.com",
    "an.facebook.com",
    // Advertising attribution / install tracking
    "mobileapptracking.com",
    "adjust.com",
    "appsflyer.com",
    "kochava.com",
    "tune.com",
];

/// Whether `host` (a name, with or without a port) belongs to a blocked
/// advertising network.
pub fn is_blocked_host(host: &str) -> bool {
    if ALLOW_ADS.load(Ordering::Relaxed) {
        return false;
    }
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    let host = host.rsplit_once(':').filter(|(_, port)| port.bytes().all(|b| b.is_ascii_digit())).map_or(host.as_str(), |(name, _)| name);
    BLOCKED_DOMAINS
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

#[cfg(test)]
mod tests {
    use super::is_blocked_host;

    #[test]
    fn blocks_ad_hosts_only() {
        assert!(is_blocked_host("ads.mopub.com"));
        assert!(is_blocked_host("10034.engine.mobileapptracking.com"));
        assert!(is_blocked_host("googleads.g.doubleclick.net:443"));
        assert!(!is_blocked_host("activeuser.qpyou.cn"));
        assert!(!is_blocked_host("notmopub.com"));
    }
}
