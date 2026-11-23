use crate::model::{atomic_json, now, read_saved_json};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const TITLE: &str = "Educational Use and Disclaimer";
pub const NOTICE: &str = "For educational use only. Do not rely on this report for actual investment or trading decisions.";
pub const TERMS: [(&str, &str); 3] = [
    (
        "Permitted use",
        "Tenerus and its forecasts, simulations, AI-generated research, and exports are provided solely for education. They do not constitute investment advice, a recommendation, or an offer to buy or sell any financial instrument. Do not use them for actual investment or trading decisions.",
    ),
    (
        "Risk and limitations",
        "Market data may be delayed, incomplete, or inaccurate. AI outputs can contain errors, and historical results and model confidence provide no assurance of future performance. Tenerus and its outputs are provided as is and as available, without warranties to the extent permitted by applicable law.",
    ),
    (
        "Limitation of liability",
        "To the fullest extent permitted by applicable law, the developer disclaims liability for losses or damages arising from use of Tenerus or reliance on its outputs, including trading and investment losses. These terms preserve all rights and liabilities that applicable law makes mandatory.",
    ),
];
pub const AGREEMENT: &str = "By choosing Agree & Continue, you accept these terms. Your continued use of Tenerus after acceptance signifies your ongoing agreement. If you disagree, quit and remove Tenerus from your device.";

#[derive(Serialize, Deserialize)]
struct Acceptance {
    accepted_at: i64,
    terms_revision: u32,
}

pub fn accepted(path: &Path) -> bool {
    read_saved_json::<Acceptance>(path)
        .ok()
        .flatten()
        .is_some_and(|receipt| receipt.accepted_at > 0 && receipt.terms_revision > 0)
}

pub fn accept(path: &Path) -> Result<(), String> {
    atomic_json(
        path,
        &Acceptance {
            accepted_at: now(),
            terms_revision: 1,
        },
    )
}
