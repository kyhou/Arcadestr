// Web entry point: Trunk/WASM build target.

use arcadestr_app::App;
use leptos::prelude::*;

fn main() {
    console_error_panic_hook::set_once();
    #[cfg(all(debug_assertions, not(feature = "web")))]
    if web_sys::window()
        .and_then(|window| window.location().search().ok())
        .is_some_and(|query| query == "?publisher-console-preview=1")
    {
        mount_to_body(|| view! {<arcadestr_app::ui_v2::views::publish::PublisherConsolePreview/>});
        return;
    }
    #[cfg(debug_assertions)]
    if web_sys::window()
        .and_then(|window| window.location().search().ok())
        .is_some_and(|query| query == "?publisher-management-preview=1")
    {
        mount_to_body(
            || view! {<arcadestr_app::components::publisher_management::PublisherManagementPreview/>},
        );
        return;
    }
    #[cfg(debug_assertions)]
    if web_sys::window()
        .and_then(|window| window.location().search().ok())
        .is_some_and(|query| query == "?publisher-recovery-preview=1")
    {
        mount_to_body(
            || view! { <arcadestr_app::components::publisher_recovery::PublisherRecoveryPreview /> },
        );
        return;
    }
    #[cfg(debug_assertions)]
    if web_sys::window()
        .and_then(|w| w.location().search().ok())
        .is_some_and(|query| query == "?receipt-portability-preview=1")
    {
        mount_to_body(|| view! {<arcadestr_app::ui_v2::receipt_sync::ReceiptPortabilityPreview/>});
        return;
    }

    #[cfg(debug_assertions)]
    if web_sys::window()
        .and_then(|w| w.location().search().ok())
        .is_some_and(|query| query == "?purchase-plan-preview=1")
    {
        mount_to_body(
            || view! {<arcadestr_app::ui_v2::components::purchase_plan::PurchasePlanPreview/>},
        );
        return;
    }

    mount_to_body(|| view! { <App/> });
}
