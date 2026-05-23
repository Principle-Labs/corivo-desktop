//! Privacy filter settings —— 持久化层接入点。
//!
//! 当前阶段:settings 暂时只在内存 (`PrivacyFilter::settings` 的
//! Arc<Mutex<...>>)。真正的持久化路径要等下一轮把 `PrivacySettings`
//! 字段挂到 `domain::config::Config` 上 + 走 `config_service` 落盘。
//!
//! 这个文件先存一份 helper —— 把 PiiLabel 一一映射到 CategoryToggles
//! 的 bool 字段的常用工具函数。放进独立 module 一方面让
//! `mod.rs` 不再臃肿,另一方面下一轮加 Tauri command 时 `commands/
//! settings.rs` 可以直接 import 这里的转换函数,不绕到 domain 去。

use crate::domain::privacy::{CategoryToggles, PiiLabel};

/// 把 PiiLabel 数组转成 CategoryToggles —— 数组里的 label 置 true,
/// 不在数组里的置 false。`secret` 永远强制 true (spec §12.1)。
///
/// 给将来的 Tauri command `set_privacy_categories(labels: Vec<PiiLabel>)`
/// 用 —— 前端发"现在用户开了哪些类目"过来,后端转成 CategoryToggles
/// 写回 settings。
pub fn toggles_from_enabled_labels(enabled: &[PiiLabel]) -> CategoryToggles {
    let has = |l: PiiLabel| enabled.contains(&l);
    CategoryToggles {
        account_number: has(PiiLabel::AccountNumber),
        private_address: has(PiiLabel::PrivateAddress),
        private_email: has(PiiLabel::PrivateEmail),
        private_person: has(PiiLabel::PrivatePerson),
        private_phone: has(PiiLabel::PrivatePhone),
        private_url: has(PiiLabel::PrivateUrl),
        private_date: has(PiiLabel::PrivateDate),
        secret: true,
    }
}

/// 反向:CategoryToggles → enabled label 列表。给前端 settings 页
/// 读当前状态时用。顺序固定,跟 spec §17 表格列序一致 —— UI 渲染
/// 时不需要额外排序。
pub fn enabled_labels_from_toggles(toggles: &CategoryToggles) -> Vec<PiiLabel> {
    let mut out = Vec::new();
    if toggles.private_person {
        out.push(PiiLabel::PrivatePerson);
    }
    if toggles.private_email {
        out.push(PiiLabel::PrivateEmail);
    }
    if toggles.private_phone {
        out.push(PiiLabel::PrivatePhone);
    }
    if toggles.private_address {
        out.push(PiiLabel::PrivateAddress);
    }
    if toggles.account_number {
        out.push(PiiLabel::AccountNumber);
    }
    if toggles.private_url {
        out.push(PiiLabel::PrivateUrl);
    }
    if toggles.private_date {
        out.push(PiiLabel::PrivateDate);
    }
    if toggles.secret {
        out.push(PiiLabel::Secret);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_from_labels_forces_secret_on() {
        let t = toggles_from_enabled_labels(&[PiiLabel::PrivatePerson]);
        assert!(t.private_person);
        assert!(t.secret, "secret must be forced on regardless of input");
    }

    #[test]
    fn toggles_from_labels_zero_input_still_keeps_secret() {
        let t = toggles_from_enabled_labels(&[]);
        assert!(!t.private_person);
        assert!(!t.private_email);
        assert!(t.secret);
    }

    #[test]
    fn round_trip_labels_via_toggles() {
        let original = vec![
            PiiLabel::PrivatePerson,
            PiiLabel::PrivateEmail,
            PiiLabel::PrivatePhone,
        ];
        let toggles = toggles_from_enabled_labels(&original);
        let restored = enabled_labels_from_toggles(&toggles);
        // 注意 round-trip 会带回 Secret(强制开),所以期望多一个。
        assert!(restored.contains(&PiiLabel::PrivatePerson));
        assert!(restored.contains(&PiiLabel::PrivateEmail));
        assert!(restored.contains(&PiiLabel::PrivatePhone));
        assert!(restored.contains(&PiiLabel::Secret));
    }
}
