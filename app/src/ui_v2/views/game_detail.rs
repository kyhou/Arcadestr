use leptos::prelude::*;

use crate::components::DetailView;
use crate::models::GameListing;

#[component]
pub fn GameDetailView(listing: GameListing, on_back: Callback<()>) -> impl IntoView {
    view! {
        <section class="v2-detail-wrap">
            <header class="v2-panel-glass v2-detail-hero" style="background-image: linear-gradient(to top, rgba(10,14,20,0.88), rgba(10,14,20,0.45)), url('https://lh3.googleusercontent.com/aida-public/AB6AXuD-Ozg13DMgznaYJPfnCPqP23kDPxch68yt6upMyXYigCQaIOMX4YdXUN3LwNGk3We8TUzv3wMIfTAquYKFk8OEvX76pBMOC_8XyhdgITa5_usQuj7BnBfaxinkhdbzYGWxwQPawXKz8ycGRM8BtW51t-mtyR--Sv20X81urEaqZQwVu17fQdfKkvNGN_bPW-Q3hwWWZ2kcScEa7h66NAkbLCmiiqwqF_qRaQ8Wsqt6sGYj3Sb2VRCzFmgOQEl-5n6kzZlQ3RfHpzU'); background-size: cover; background-position: center;">
                <div>
                    <p class="v2-store-kicker">"Masterpiece Edition"</p>
                    <h1 class="v2-display v2-detail-title">{listing.title.clone()}</h1>
                    <div class="v2-detail-rating-row">
                        <span>"star star star star star_half"</span>
                        <span>"4.8"</span>
                        <span>"|"</span>
                        <span>"bolt 12.4k Zaps"</span>
                    </div>
                    <p class="v2-hero-description">{listing.description.clone()}</p>
                    <div class="v2-detail-tags">
                        {listing
                            .tags
                            .iter()
                            .take(4)
                            .map(|tag| view! { <span class="v2-chip">{tag.clone()}</span> })
                            .collect::<Vec<_>>()}
                    </div>
                </div>
                <aside class="v2-detail-buy-panel v2-panel">
                    <div class="v2-detail-price">"84k Sats"</div>
                    <p class="v2-social-meta">"120k"</p>
                    <button class="v2-btn-secondary" on:click=move |_| on_back.run(())>
                        "Back"
                    </button>
                    <button class="v2-btn-primary">
                        "Buy with Lightning"
                    </button>
                    <button class="v2-btn-ghost">"Add to Library"</button>
                    <p class="v2-social-meta">"Developer: Luminescent Labs"</p>
                    <p class="v2-social-meta">"Publisher: Arcade Vault"</p>
                    <p class="v2-social-meta">"Release Date: Oct 24, 2023"</p>
                    <p class="v2-social-meta">"Protocol: NIP-01 / NIP-57"</p>

                    <section class="v2-detail-currently-playing">
                        <h4>"Currently Playing"</h4>
                        <div class="v2-playing-row">
                            <span>"SatoshiGamer"</span>
                            <span>"Streaming"</span>
                        </div>
                        <div class="v2-playing-row">
                            <span>"PlebsOnly"</span>
                            <span>"Level 12"</span>
                        </div>
                    </section>
                </aside>
            </header>

            <section class="v2-panel v2-detail-description-block">
                <h2 class="v2-display">"The Final Protocol"</h2>
                <p>
                    "Dive into the sprawling mega-city of Aetheria. As a rogue data-jockey, you must navigate the digital underbelly of a world where memories are currency and identity is a luxury. Neural Shift: 2099 features real-time physics, branching narrative, and a fully player-driven protocol economy."
                </p>
                <div class="v2-detail-tags">
                    <span class="v2-chip">"Cyberpunk"</span>
                    <span class="v2-chip">"RPG"</span>
                    <span class="v2-chip">"Multiplayer"</span>
                    <span class="v2-chip">"Open World"</span>
                </div>
            </section>

            <div class="v2-detail-grid">
                <section class="v2-panel-glass v2-detail-feed">
                    <div class="v2-detail-gallery-grid">
                        <img src="https://lh3.googleusercontent.com/aida-public/AB6AXuAcfBB9wVWQRzbVzZNxtNioKH61w-X4MmIXHTE15-JLyPDa1IAN-fSbXI3o0fs0XDYlGid3IQ1mAfMFvfx2ZF7xpTYW_4kpq8HAni29hcKe4u_RJADpbfHcPPxUHen0DwNoH2NytKmvYwJ60ZxZ3AKjIKlaQOVv_ErtppJmrWxWTCyrv-cmFzLFa9j-u-pEjN4xJ2iauGJ7ZAjrvKpd4DIdJ-VOAbmg5hYup4DJbJ-tH9yjU9j66X6oBOcsmMJwX9URXYKkBdk0iMw" alt="media one" />
                        <img src="https://lh3.googleusercontent.com/aida-public/AB6AXuBVdFhFtMzOVXqb-BI5LIc-DCj7ZWW7fgs9duy1L-jfclptfUAblLqB_Y6xiqndytreddf-sMbPbNJzl4lrydoIO6HnSUqEpBwViZ2Pp8ntP1U4K8NMemUaCCQzXzerOi-KbzgADk2tr3IsZD4jPIHHlLJY4JcU9f7wYKGFZK9PXLYKuOqICzbhI_R2A_dYff79Yre8zQsKbmjn_CDIz5oGoYdsjnze17WY7rCT1hhY17ku2L-imraqcq2jLCwkslUsOf2sdSg4wkc" alt="media two" />
                        <img src="https://lh3.googleusercontent.com/aida-public/AB6AXuA6idpKB3GVhrNo5YFYN-jQumE7snn3zDjMFANFAWQuk5SMhre2nS4CQnOkuF7jW1OCnJ21J2nDddQ_MiWkmNdhjCYBdn_3bPs7MSAL7JvJtTEqJFu1Ax3r6b2TGstD986OoYSaE-KlZvL7WCg_6R3-LNFzQcecCtlaq6-qnip0FXUrMK1zKKSaG5-LIDEHGf5NKssIGxgJxd-3OXlSoJqtmRyUtsDIHFGTtaZLvKuYDnDKiBoJBIqrMHPpvGeyVDoBpuZNoYW8eDE" alt="media three" />
                    </div>

                    <div class="v2-section-header">
                        <h3>"Nostr Feed"</h3>
                        <button class="v2-btn-ghost">"Write a Note"</button>
                    </div>
                    <div class="v2-live-note v2-detail-note-card">
                        <p class="v2-social-meta">"npub1...k9q2 - 2h ago"</p>
                        <p>"Level progression feels balanced. Definitely worth the sats."</p>
                        <div class="v2-social-actions">
                            <span>"bolt 1,240"</span>
                            <span>"chat 12"</span>
                            <span>"sync 45"</span>
                        </div>
                    </div>
                    <div class="v2-live-note v2-detail-note-card">
                        <p class="v2-social-meta">"npub1...r5z8 - 5h ago"</p>
                        <p>"Soundtrack quality is outstanding. Zapped this review for visibility."</p>
                        <div class="v2-social-actions">
                            <span>"bolt 890"</span>
                            <span>"chat 4"</span>
                            <span>"sync 18"</span>
                        </div>
                    </div>
                </section>

                <section class="v2-panel v2-detail-specs">
                    <h3>"Specs"</h3>
                    <div class="v2-spec-grid">
                        <span>"OS"</span>
                        <span>"Linux / Win 11"</span>
                        <span>"GPU"</span>
                        <span>"RTX 3070+"</span>
                        <span>"Storage"</span>
                        <span>"85 GB SSD"</span>
                    </div>
                </section>
            </div>

            <section class="v2-panel v2-detail-transaction-wrap">
                <DetailView listing={listing} on_back={on_back} />
            </section>
        </section>
    }
}
