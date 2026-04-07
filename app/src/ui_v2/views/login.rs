use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::components::BackupManager;
use crate::ui_v2::theme::UI_V2_STYLES;
use crate::{
    invoke_check_qr_connection, invoke_connect_bunker, invoke_connect_nip07,
    invoke_generate_nostrconnect_uri, invoke_has_accounts, invoke_start_qr_login, AuthContext,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddAccountMode {
    Methods,
    Qr,
    Restore,
}

#[component]
pub fn LoginV2View() -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");
    let auth_stored = StoredValue::new(auth.clone());

    let show_add_account = RwSignal::new(false);
    let add_mode = RwSignal::new(AddAccountMode::Methods);

    let bunker_uri = RwSignal::new(String::new());
    let bunker_display_name = RwSignal::new(String::new());
    let relay = RwSignal::new("wss://relay.damus.io".to_string());
    let generated_uri = RwSignal::new(None::<String>);

    let nsec_input = RwSignal::new(String::new());
    let name_input = RwSignal::new(String::new());

    let qr_uri = RwSignal::new(None::<String>);
    let qr_loading = RwSignal::new(false);
    let qr_error = RwSignal::new(None::<String>);
    let qr_polling = RwSignal::new(false);

    Effect::new(move |_| {
        let auth = auth_stored.get_value();
        spawn_local(async move {
            let has_accounts = invoke_has_accounts().await.unwrap_or(false);
            if has_accounts {
                let _ = auth.load_accounts_list().await;
                show_add_account.set(false);
            } else {
                show_add_account.set(true);
            }
            add_mode.set(AddAccountMode::Methods);
        });
    });

    let on_connect_bunker = move |_| {
        let auth = auth_stored.get_value();
        let uri_val = bunker_uri.get();
        let display_name_val = bunker_display_name.get();

        if uri_val.is_empty() {
            auth.error
                .set(Some("Please enter a bunker URI or NIP-05 identifier".to_string()));
            return;
        }

        if auth.is_loading.get() {
            return;
        }

        auth.is_loading.set(true);
        auth.error.set(None);

        spawn_local(async move {
            match invoke_connect_bunker(uri_val, display_name_val).await {
                Ok(result) => {
                    if let Some(npub) = result.get("pubkey").and_then(|v| v.as_str()) {
                        let _ = auth.load_profiles_list().await;
                        let _ = auth.load_accounts_list().await;
                        auth.npub.set(Some(npub.to_string()));
                        auth.has_secure_accounts.set(true);
                        auth.is_loading.set(false);
                        auth.start_connection_status_polling().await;
                        show_add_account.set(false);
                        bunker_uri.set(String::new());
                        bunker_display_name.set(String::new());
                    } else {
                        auth.error
                            .set(Some("Connected but no pubkey in response".to_string()));
                        auth.is_loading.set(false);
                    }
                }
                Err(e) => {
                    auth.error.set(Some(format!("Failed to connect: {e}")));
                    auth.is_loading.set(false);
                }
            }
        });
    };

    let on_connect_nip07 = move |_: leptos::ev::MouseEvent| {
        let auth = auth_stored.get_value();
        auth.is_loading.set(true);
        auth.error.set(None);

        spawn_local(async move {
            match invoke_connect_nip07().await {
                Ok(npub) => {
                    let _ = auth.load_profiles_list().await;
                    let _ = auth.load_accounts_list().await;
                    auth.npub.set(Some(npub));
                    auth.has_secure_accounts.set(true);
                    auth.is_loading.set(false);
                    show_add_account.set(false);
                }
                Err(e) => {
                    auth.error
                        .set(Some(format!("Failed to connect NIP-07 signer: {e}")));
                    auth.is_loading.set(false);
                }
            }
        });
    };

    let on_generate_nostrconnect = move |_| {
        let auth = auth_stored.get_value();
        let relay_val = relay.get();
        auth.is_loading.set(true);
        auth.error.set(None);

        spawn_local(async move {
            match invoke_generate_nostrconnect_uri(relay_val).await {
                Ok(result) => {
                    if let Some(uri) = result.get("uri").and_then(|v| v.as_str()) {
                        generated_uri.set(Some(uri.to_string()));
                    }
                    auth.is_loading.set(false);
                }
                Err(e) => {
                    auth.error
                        .set(Some(format!("Failed to generate nostrconnect URI: {e}")));
                    auth.is_loading.set(false);
                }
            }
        });
    };

    let on_nsec_login = move |_| {
        let auth = auth_stored.get_value();
        let nsec = nsec_input.get();
        let name = name_input.get();

        if nsec.is_empty() {
            return;
        }

        auth.error.set(None);

        spawn_local(async move {
            let name_opt = if name.is_empty() { None } else { Some(name) };
            match auth.login_with_nsec(nsec, name_opt).await {
                Ok(_) => {
                    let _ = auth.load_accounts_list().await;
                    show_add_account.set(false);
                    add_mode.set(AddAccountMode::Methods);
                    nsec_input.set(String::new());
                    name_input.set(String::new());
                }
                Err(e) => auth.error.set(Some(e)),
            }
        });
    };

    let on_start_qr_login = move |_| {
        let auth = auth_stored.get_value();
        qr_loading.set(true);
        qr_error.set(None);

        spawn_local(async move {
            match invoke_start_qr_login().await {
                Ok(uri) => {
                    qr_uri.set(Some(uri));
                    qr_loading.set(false);
                    add_mode.set(AddAccountMode::Qr);
                    qr_polling.set(true);

                    let auth_for_poll = auth.clone();
                    spawn_local(async move {
                        while qr_polling.get() {
                            match invoke_check_qr_connection().await {
                                Ok(Some(result)) => {
                                    if let Some(npub) = result.get("pubkey").and_then(|v| v.as_str()) {
                                        let _ = auth_for_poll.load_profiles_list().await;
                                        let _ = auth_for_poll.load_accounts_list().await;
                                        auth_for_poll.npub.set(Some(npub.to_string()));
                                        auth_for_poll.has_secure_accounts.set(true);
                                    }
                                    qr_polling.set(false);
                                    show_add_account.set(false);
                                    add_mode.set(AddAccountMode::Methods);
                                    break;
                                }
                                Ok(None) => {}
                                Err(e) => {
                                    qr_error.set(Some(e));
                                    qr_polling.set(false);
                                    break;
                                }
                            }

                            #[cfg(target_arch = "wasm32")]
                            {
                                use js_sys::Promise;
                                use wasm_bindgen_futures::JsFuture;
                                let _ = JsFuture::from(Promise::new(&mut |resolve, _| {
                                    web_sys::window()
                                        .expect("window available")
                                        .set_timeout_with_callback_and_timeout_and_arguments_0(
                                            &resolve, 5000,
                                        )
                                        .expect("set timeout");
                                }))
                                .await;
                            }
                        }
                    });
                }
                Err(e) => {
                    qr_error.set(Some(e));
                    qr_loading.set(false);
                }
            }
        });
    };

    let on_cancel_add = move |_| {
        qr_polling.set(false);
        qr_uri.set(None);
        qr_error.set(None);
        add_mode.set(AddAccountMode::Methods);
        spawn_local(async move {
            if invoke_has_accounts().await.unwrap_or(false) {
                show_add_account.set(false);
            }
        });
    };

    let account_cards = Signal::derive(move || auth.accounts.get());
    let manual_connect_uri = Signal::derive(move || {
        generated_uri
            .get()
            .or_else(|| qr_uri.get())
            .unwrap_or_else(|| "nostrconnect://823fe...431".to_string())
    });
    let active_qr_uri = Signal::derive(move || qr_uri.get().or_else(|| generated_uri.get()));

    view! {
        <div class="bg-background text-on-surface font-body selection:bg-primary/30 min-h-screen overflow-x-hidden">
            <style>{UI_V2_STYLES}</style>

            {move || {
                if show_add_account.get() {
                    view! {
                        <section class="v2-add-account-screen min-h-screen bg-background text-on-surface font-body">
                            <nav class="sticky top-0 z-50 px-6 py-4 flex items-center justify-between bg-background/80 backdrop-blur-md">
                                <button class="flex items-center gap-2 text-on-surface hover:text-primary transition-colors" on:click=on_cancel_add>
                                    <span class="material-symbols-outlined">"arrow_back"</span>
                                    <span class="font-label text-sm uppercase tracking-widest font-bold">"Cancel"</span>
                                </button>
                                <div class="flex items-center gap-3">
                                    <div class="w-6 h-6 text-primary inline-flex items-center justify-center">
                                        <svg class="w-full h-full" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48" fill="currentColor">
                                            <path d="M44 4H30.6666V17.3334H17.3334V30.6666H4V44H44V4Z"></path>
                                        </svg>
                                    </div>
                                    <span class="font-headline text-lg font-bold">"Arcadestr Noir"</span>
                                </div>
                                <div class="w-10"></div>
                            </nav>

                            <main class="max-w-4xl mx-auto px-6 py-12">
                                <header class="mb-12 text-center md:text-left">
                                    <h1 class="font-headline text-5xl md:text-6xl font-black tracking-tight mb-4 text-on-surface">
                                        "Connect "<span class="text-primary">"Nostr"</span>
                                    </h1>
                                    <p class="text-on-surface-variant text-lg max-w-xl">
                                        "Bridge your decentralized identity to the ultimate gaming curator. Choose your preferred connection method below."
                                    </p>
                                </header>

                                <Show when=move || add_mode.get() == AddAccountMode::Methods || add_mode.get() == AddAccountMode::Qr>
                                    <div class="grid grid-cols-1 lg:grid-cols-12 gap-8">
                                        <section class="lg:col-span-7 space-y-6">
                                            <div class="glass-panel p-8 rounded-xl">
                                                <div class="flex items-center gap-3 mb-6">
                                                    <span class="material-symbols-outlined text-secondary" style="font-variation-settings: 'FILL' 1;">"security"</span>
                                                    <h2 class="font-headline text-2xl font-bold">"Nostr Bunker"</h2>
                                                </div>
                                                <p class="text-on-surface-variant mb-6 text-sm">"Use a remote signer to keep your keys safe on your mobile device or specialized hardware."</p>
                                                <div class="space-y-4">
                                                    <div class="group">
                                                        <label class="block text-label text-sm font-medium mb-2 text-on-surface">"Relay URL"</label>
                                                        <input
                                                            class="w-full bg-surface-container-highest border-none rounded-md px-4 py-4 text-on-surface placeholder:text-on-surface-variant/50 focus:ring-2 focus:ring-secondary/40 transition-all outline-none"
                                                            placeholder="wss://relay.damus.io"
                                                            prop:value=move || relay.get()
                                                            on:input:target=move |ev| relay.set(ev.target().value())
                                                            type="text"
                                                        />
                                                    </div>
                                                    <div class="group">
                                                        <label class="block text-label text-sm font-medium mb-2 text-on-surface">"Bunker URL"</label>
                                                        <input
                                                            class="w-full bg-surface-container-highest border-none rounded-md px-4 py-4 text-on-surface placeholder:text-on-surface-variant/50 focus:ring-2 focus:ring-secondary/40 transition-all outline-none"
                                                            placeholder="bunker://<npub>@relay.example.com or user@nsec.app"
                                                            prop:value=move || bunker_uri.get()
                                                            on:input:target=move |ev| bunker_uri.set(ev.target().value())
                                                            type="text"
                                                        />
                                                    </div>
                                                    <input
                                                        class="w-full bg-surface-container-highest border-none rounded-md px-4 py-4 text-on-surface placeholder:text-on-surface-variant/50 focus:ring-2 focus:ring-secondary/40 transition-all outline-none"
                                                        placeholder="Profile Name (optional)"
                                                        prop:value=move || bunker_display_name.get()
                                                        on:input:target=move |ev| bunker_display_name.set(ev.target().value())
                                                        type="text"
                                                    />
                                                    <button
                                                        class="w-full bg-gradient-to-r from-primary to-primary-dim text-on-primary-fixed py-4 rounded-md font-headline font-bold text-lg hover:brightness-110 active:scale-[0.98] transition-all shadow-lg shadow-primary-dim/20"
                                                        on:click=on_connect_bunker
                                                        disabled=move || auth_stored.get_value().is_loading.get()
                                                    >
                                                        {move || if auth_stored.get_value().is_loading.get() { "Connecting..." } else { "Connect Bunker" }}
                                                    </button>
                                                </div>
                                            </div>

                                            <div class="glass-panel p-8 rounded-xl">
                                                <div class="flex items-center gap-3 mb-4">
                                                    <span class="material-symbols-outlined text-error-dim">"key"</span>
                                                    <h2 class="font-headline text-xl font-bold">"Direct nsec Entry"</h2>
                                                </div>
                                                <div class="bg-error-container/10 border border-error-dim/20 rounded-lg p-4 mb-6">
                                                    <div class="flex gap-3">
                                                        <span class="material-symbols-outlined text-error-dim text-sm">"warning"</span>
                                                        <p class="text-xs text-error-dim/80 leading-relaxed">
                                                            <strong class="text-error-dim">"CRITICAL WARNING:"</strong>
                                                            " Entering your "<code class="bg-error-dim/20 px-1 rounded">"nsec"</code>
                                                            " directly is not recommended. This stores your private key in the browser. Only use this for burner accounts or trusted environments."
                                                        </p>
                                                    </div>
                                                </div>
                                                <input
                                                    class="w-full bg-surface-container-lowest border border-outline-variant/10 rounded-md px-4 py-3 text-on-surface placeholder:text-on-surface-variant/30 focus:ring-1 focus:ring-error-dim transition-all outline-none mb-3"
                                                    placeholder="Account name (optional)"
                                                    bind:value=name_input
                                                    type="text"
                                                />
                                                <div class="flex gap-3">
                                                    <input
                                                        class="flex-1 bg-surface-container-lowest border border-outline-variant/10 rounded-md px-4 py-3 text-on-surface placeholder:text-on-surface-variant/30 focus:ring-1 focus:ring-error-dim transition-all outline-none"
                                                        placeholder="nsec1..."
                                                        bind:value=nsec_input
                                                        type="password"
                                                    />
                                                    <button
                                                        class="bg-surface-variant hover:bg-surface-bright text-on-surface px-6 py-3 rounded-md font-bold text-sm transition-[background-color] duration-300 ease-out motion-safe:will-change-transform"
                                                        on:click=on_nsec_login
                                                        disabled=move || nsec_input.get().is_empty() || auth_stored.get_value().is_loading.get()
                                                    >
                                                        "Import"
                                                    </button>
                                                </div>
                                            </div>
                                        </section>

                                        <section class="lg:col-span-5 space-y-6">
                                            <div class="glass-panel p-8 rounded-xl flex flex-col items-center text-center">
                                                <div class="flex items-center gap-3 mb-6 w-full justify-start">
                                                    <span class="material-symbols-outlined text-tertiary">"qr_code_2"</span>
                                                    <h2 class="font-headline text-2xl font-bold">"nostrconnect"</h2>
                                                </div>

                                                <div class="relative group cursor-pointer mb-8">
                                                    <Show
                                                        when=move || active_qr_uri.get().is_some()
                                                        fallback=move || {
                                                            view! {
                                                                <div class="bg-white p-4 rounded-xl shadow-2xl w-56 h-56 flex items-center justify-center text-center">
                                                                    <p class="text-xs text-on-surface-variant px-4">
                                                                        "Generate a nostrconnect URI or start QR login to show the live QR code."
                                                                    </p>
                                                                </div>
                                                            }
                                                        }
                                                    >
                                                        {move || {
                                                            active_qr_uri.get().map(|uri| {
                                                                let qr_svg = crate::qr::generate_qr_svg(&uri);
                                                                view! {
                                                                    <div class="bg-white p-4 rounded-xl shadow-2xl transition-transform group-hover:scale-[1.02]" aria-label="Nostr Connect QR Code">
                                                                        <div class="v2-dynamic-qr" inner_html=qr_svg></div>
                                                                    </div>
                                                                }
                                                            })
                                                        }}
                                                    </Show>
                                                    <div class="absolute inset-0 bg-background/20 rounded-xl pointer-events-none"></div>
                                                </div>

                                                <p class="text-sm text-on-surface-variant mb-4">
                                                    "Scan with your mobile signer (e.g., Amethyst, Alby) to authorize Arcadestr Noir."
                                                </p>

                                                <div class="w-full space-y-3">
                                                    <label class="block text-left text-label text-xs uppercase tracking-tighter font-bold mb-2 text-on-surface-variant">"Manual Connection String"</label>
                                                    <div class="flex items-center bg-surface-container-lowest rounded-md border border-outline-variant/10 p-1">
                                                        <input class="bg-transparent border-none focus:ring-0 text-xs text-on-surface/60 flex-1 px-3 py-2 truncate" type="text" readonly=true prop:value=move || manual_connect_uri.get() />
                                                        <button
                                                            class="bg-surface-container-high hover:bg-surface-bright text-primary p-2 rounded-md transition-[background-color] duration-300 ease-out motion-safe:will-change-transform"
                                                            title="Copy to clipboard"
                                                            on:click=move |_| {
                                                                #[cfg(target_arch = "wasm32")]
                                                                {
                                                                    if let Some(window) = web_sys::window() {
                                                                        let _ = window.navigator().clipboard().write_text(&manual_connect_uri.get());
                                                                    }
                                                                }
                                                            }
                                                        >
                                                            <span class="material-symbols-outlined text-base">"content_copy"</span>
                                                        </button>
                                                    </div>

                                                    <button class="w-full bg-surface-container-high hover:bg-surface-bright text-on-surface py-3 rounded-md font-bold text-sm" on:click=on_generate_nostrconnect>
                                                        "Generate nostrconnect URI"
                                                    </button>

                                                    <button class="w-full bg-surface-container-high hover:bg-surface-bright text-on-surface py-3 rounded-md font-bold text-sm" on:click=on_start_qr_login disabled=move || qr_loading.get()>
                                                        {move || if qr_loading.get() { "Generating..." } else { "Start QR Login" }}
                                                    </button>

                                                    {#[cfg(feature = "web")]
                                                        view! {
                                                            <button
                                                                class="w-full bg-surface-container-high hover:bg-surface-bright text-on-surface py-3 rounded-md font-bold text-sm"
                                                                on:click=on_connect_nip07
                                                                disabled=move || auth_stored.get_value().is_loading.get()
                                                            >
                                                                "Connect NIP-07"
                                                            </button>
                                                        }
                                                    }

                                                    <button class="w-full bg-surface-container-high hover:bg-surface-bright text-on-surface py-3 rounded-md font-bold text-sm" on:click=move |_| add_mode.set(AddAccountMode::Restore)>
                                                        "Restore Backup"
                                                    </button>
                                                </div>

                                                <Show when=move || add_mode.get() == AddAccountMode::Qr>
                                                    <div class="w-full mt-4 p-3 rounded-md bg-surface-container-lowest text-left">
                                                        <p class="text-xs text-tertiary">"Waiting for QR signer connection..."</p>
                                                        <Show when=move || qr_error.get().is_some()>
                                                            <p class="text-xs text-error-dim mt-2">{move || qr_error.get().unwrap_or_default()}</p>
                                                        </Show>
                                                    </div>
                                                </Show>
                                            </div>

                                            <div class="p-6 rounded-xl border border-outline-variant/5 bg-surface-container-low/30">
                                                <h3 class="font-headline font-bold text-sm mb-2 text-on-surface flex items-center gap-2">
                                                    <span class="material-symbols-outlined text-secondary-dim text-sm">"help"</span>
                                                    "New to Nostr?"
                                                </h3>
                                                <p class="text-xs text-on-surface-variant leading-relaxed">
                                                    "Nostr is a decentralized protocol for social media and beyond. Unlike traditional accounts, you own your identity via cryptographic keys. "
                                                    <a class="text-secondary hover:underline" href="#">"Learn more about keys and security."</a>
                                                </p>
                                            </div>
                                        </section>
                                    </div>
                                </Show>

                                <Show when=move || add_mode.get() == AddAccountMode::Restore>
                                    <section class="glass-panel p-8 rounded-xl">
                                        <div class="flex items-center justify-between mb-4">
                                            <h2 class="font-headline text-2xl font-bold">"Restore from Backup"</h2>
                                            <button class="text-on-surface-variant hover:text-on-surface" on:click=move |_| add_mode.set(AddAccountMode::Methods)>
                                                "Back"
                                            </button>
                                        </div>
                                        <BackupManager />
                                    </section>
                                </Show>

                                <footer class="mt-20 pt-10 border-t border-outline-variant/10 flex flex-col md:flex-row justify-between items-center gap-6 opacity-60 grayscale hover:grayscale-0 transition-all duration-700">
                                    <div class="flex gap-8 items-center">
                                        <img class="h-6 object-contain" alt="Partner Logo" src="https://lh3.googleusercontent.com/aida-public/AB6AXuA1WMGTX4twJv1WRaQRs-SLDogS3afkrryX94MjYTKh8GpZchXClvnzAf1_2_jS4x4GeSOcrrBJaB-CfRuO3Ho-RB0pEvDsw0owNn64U3qH2SB8lZR9DfA1iqO-gQA1Xpztq5In9uoG-MmlbhQ0ZbyAoWQDrCOj4qfnhVnB_YndWhJ6W6uvaFULrKLcv2qu3kqyVvsm6UXjlUV0Rttlyj3w3FEUurwta-rudMVfK77DA3IBcq8gSS3WZncVe1BxhEJVUM7Qfl706GQ" />
                                        <img class="h-6 object-contain" alt="Partner Logo" src="https://lh3.googleusercontent.com/aida-public/AB6AXuD-8HeRDZN0ZpoSE5IWzt6UmNWqqIzFTNMYDmRwRNHWNpt6Vm2pgHfSMJ40JAjjqDwk5NZ3_Z2kkejSXfjiSrMmQOqoDi8BekQ0nRVhbbZmIdjtjQKJzd7SaawUH4Opx1xpx-so1dVr8fHjSWJIZE3sobmENuWE4hMGuPaAyGrT5d7g0bEigZ2_GVmd5ezMggTnDI8imK-oFCJ6lwsXrLpixGmX1zFeFbdwoXz6hQ3z_cVgc7mjqxvhIIUP9NQaxc3-xfCogIuTcVk" />
                                    </div>
                                    <p class="text-[10px] font-label uppercase tracking-[0.2em] text-on-surface-variant">"Arcadestr Noir © 2024 • Decentralized Curator Framework"</p>
                                </footer>
                            </main>
                        </section>
                    }
                        .into_any()
                } else {
                    view! {
                        <main class="min-h-screen relative flex flex-col items-center justify-center p-6 md:p-12">
                            <div class="v2-login-glow v2-login-glow-left"></div>
                            <div class="v2-login-glow v2-login-glow-right"></div>

                            <section class="w-full max-w-4xl glass-panel rounded-xl p-8 md:p-12 border border-outline-variant/10 shadow-[0px_20px_40px_rgba(0,0,0,0.4)]">
                                <header class="text-center mb-12">
                                    <h1 class="font-headline text-4xl md:text-5xl font-bold tracking-tight mb-4 text-on-surface">"Who's playing today?"</h1>
                                    <p class="text-on-surface-variant font-body text-lg">"Switch to an existing profile or connect a new identity."</p>
                                </header>

                                <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-6 mb-12">
                                    {move || {
                                        account_cards
                                            .get()
                                            .into_iter()
                                            .map(|acc| {
                                                let auth_for_switch = auth_stored.get_value();
                                                let auth_for_delete = auth_stored.get_value();
                                                let account_id = acc.id.clone();
                                                let account_id_for_delete = acc.id.clone();
                                                let account_label = acc
                                                    .display_name
                                                    .clone()
                                                    .or(acc.username.clone())
                                                    .or(acc.name.clone())
                                                    .unwrap_or(acc.npub.clone());
                                                let account_avatar = acc.picture.clone();
                                                let is_current = acc.is_current;
                                                let avatar_fallback = account_label
                                                    .chars()
                                                    .next()
                                                    .map(|c| c.to_ascii_uppercase().to_string())
                                                    .unwrap_or_else(|| "?".to_string());
                                                let npub_short = if acc.npub.len() > 16 {
                                                    format!(
                                                        "{}...{}",
                                                        &acc.npub[..8],
                                                        &acc.npub[acc.npub.len() - 8..]
                                                    )
                                                } else {
                                                    acc.npub.clone()
                                                };
                                                let (mode_label, mode_tone_class) =
                                                    match acc.signing_mode.to_ascii_lowercase().as_str()
                                                    {
                                                        "nip46" => (
                                                            "Connected via NIP-46".to_string(),
                                                            "text-secondary",
                                                        ),
                                                        "local" => (
                                                            "nsec (Local)".to_string(),
                                                            "text-on-surface-variant",
                                                        ),
                                                        _ => (acc.signing_mode.clone(), "text-tertiary"),
                                                    };
                                                let card_class = format!(
                                                    "group relative bg-surface-container-high hover:bg-surface-bright rounded-xl p-6 transition-[background-color,border-color] duration-300 ease-out border cursor-pointer motion-safe:will-change-transform {}",
                                                    if is_current {
                                                        "border-primary/40"
                                                    } else {
                                                        "border-transparent hover:border-primary/30"
                                                    }
                                                );
                                                let mode_class = format!(
                                                    "mt-2 inline-block px-3 py-1 rounded-sm bg-surface-container-lowest text-[10px] font-bold tracking-wider uppercase {}",
                                                    mode_tone_class
                                                );

                                                view! {
                                                    <div
                                                        class={card_class}
                                                        on:click=move |_| {
                                                            if !is_current {
                                                                let auth = auth_for_switch.clone();
                                                                let account_id = account_id.clone();
                                                                spawn_local(async move {
                                                                    let _ = auth.switch_account(account_id).await;
                                                                });
                                                            }
                                                        }
                                                    >
                                                        <button
                                                            class="absolute right-3 top-3 text-on-surface-variant hover:text-error-dim"
                                                            title="Delete account"
                                                            on:click=move |ev| {
                                                                ev.stop_propagation();
                                                                let auth = auth_for_delete.clone();
                                                                let account_id = account_id_for_delete.clone();
                                                                spawn_local(async move {
                                                                    let _ = auth.delete_account(account_id).await;
                                                                });
                                                            }
                                                        >
                                                            <span class="material-symbols-outlined">"delete"</span>
                                                        </button>

                                                        <div class="w-full text-center flex flex-col items-center">
                                                            <div class="relative mb-4">
                                                                {match account_avatar {
                                                                    Some(url) => view! {
                                                                        <img
                                                                            src={url}
                                                                            alt="Profile avatar"
                                                                            class={if is_current {
                                                                                "w-24 h-24 rounded-full p-1 bg-gradient-to-tr from-primary to-secondary object-cover"
                                                                            } else {
                                                                                "w-24 h-24 rounded-full p-1 bg-outline-variant object-cover"
                                                                            }}
                                                                        />
                                                                    }
                                                                        .into_any(),
                                                                    None => view! {
                                                                        <div class={if is_current {
                                                                            "w-24 h-24 rounded-full p-1 bg-gradient-to-tr from-primary to-secondary"
                                                                        } else {
                                                                            "w-24 h-24 rounded-full p-1 bg-outline-variant"
                                                                        }}>
                                                                            <div class="w-full h-full rounded-full bg-surface-container-highest flex items-center justify-center text-3xl font-headline font-bold text-on-surface">{avatar_fallback}</div>
                                                                        </div>
                                                                    }
                                                                        .into_any(),
                                                                }}

                                                                <Show when=move || is_current>
                                                                    <div class="absolute -bottom-1 -right-1 bg-primary w-6 h-6 rounded-full flex items-center justify-center text-[10px] text-on-primary">
                                                                        <span class="material-symbols-outlined text-xs" style="font-variation-settings: 'FILL' 1;">"check"</span>
                                                                    </div>
                                                                </Show>
                                                            </div>

                                                            <h3 class="font-headline text-lg font-bold text-on-surface group-hover:text-primary transition-colors">{account_label}</h3>
                                                            <p class="mt-1 text-xs text-on-surface-variant">{npub_short}</p>
                                                            <span class={mode_class}>{mode_label}</span>
                                                        </div>
                                                    </div>
                                                }
                                            })
                                            .collect::<Vec<_>>()
                                    }}

                                    <button class="group relative bg-surface-container-low border-2 border-dashed border-outline-variant/30 hover:border-primary/50 hover:bg-surface-container-high rounded-xl p-6 transition-all duration-300 cursor-pointer flex flex-col items-center justify-center text-center" on:click=move |_| show_add_account.set(true)>
                                        <div class="w-16 h-16 rounded-full bg-surface-container-highest flex items-center justify-center mb-4 group-hover:scale-110 transition-transform">
                                            <span class="material-symbols-outlined v2-icon-30 text-3xl text-primary">"add"</span>
                                        </div>
                                        <h3 class="font-headline text-lg font-bold text-on-surface-variant group-hover:text-on-surface transition-colors">"Add New Account"</h3>
                                    </button>
                                </div>
                            </section>
                        </main>
                    }
                        .into_any()
                }
            }}

            <Show when=move || auth_stored.get_value().error.get().is_some()>
                <div class="fixed bottom-4 right-4 max-w-md px-4 py-3 rounded-lg bg-error-container text-on-error-container border border-error-dim/30 z-[110]">
                    {move || auth_stored.get_value().error.get().unwrap_or_default()}
                </div>
            </Show>
        </div>
    }
}
