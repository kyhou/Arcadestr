use leptos::prelude::*;

#[component]
pub fn SocialView() -> impl IntoView {
    view! {
        <section class="v2-social-grid">
            <header class="v2-social-hero">
                <div class="v2-panel v2-social-composer-card">
                    <div class="v2-social-composer-head">
                        <img class="v2-social-composer-avatar" src="https://lh3.googleusercontent.com/aida-public/AB6AXuBsdwyCaCyKBYTp1vlwhvYLaiU34fCVQBxr0duzl0Naj5yw7pPL4NvBpK125cLG-BPatOq65qc1MDDjR1L8d6AlxgVKxEfapUHmoqRY-QhT00xU_neIwO1P3oS2dL0qpTBK4YWfFIvm9y94JCWrrfSJhxaLTV2F4JBLWRWcTDFIzS9U2i8LO5IfmrquUSh1wAXLv35UPPWOg1TLwGcYVD9ZZUiS9lvGkG3jIyYj6HcIAQAYxbQ6HrdyaofHsPnVIKDxGyUpu5M8SuQ" alt="profile" />
                        <textarea class="v2-social-composer-text" placeholder="Share a game review or protocol update..."></textarea>
                    </div>
                    <div class="v2-social-composer-actions">
                        <div class="v2-social-composer-tools">
                            <button class="v2-icon-btn" title="Image"><span class="material-symbols-outlined">"image"</span></button>
                            <button class="v2-icon-btn" title="Gif"><span class="material-symbols-outlined">"gif_box"</span></button>
                            <button class="v2-icon-btn" title="Poll"><span class="material-symbols-outlined">"poll"</span></button>
                            <button class="v2-icon-btn" title="Emoji"><span class="material-symbols-outlined">"sentiment_satisfied"</span></button>
                        </div>
                        <button class="v2-btn-primary">"Broadcast Note"</button>
                    </div>
                </div>
            </header>

            <div class="v2-social-layout-grid">
                <section class="v2-social-main">
                    <div class="v2-section-header v2-social-feed-header">
                        <h2 class="v2-display">"Protocol Feed"</h2>
                        <div class="v2-tab-row">
                            <button class="v2-tab active">"All Notes"</button>
                            <button class="v2-tab">"Long Form"</button>
                        </div>
                    </div>

                    <article class="v2-panel-glass v2-social-card">
                        <img class="v2-social-hero-media" src="https://lh3.googleusercontent.com/aida-public/AB6AXuCVeQFTW3HmI-oZ-A1XejD3tmRmOrpPc270JbipGPqVWt9lnFKG7rJLwFP93YEy8Y3V2EyGmrXUz4IWZIfeMX__-O3plK1EuuHluep6cwMndxLeQ70ubbX8HH7T1-v-Mz8tatZQDqxMJ1zCavgQVo8hF-lO8CFg5hqt7rJJe3ZUwPOPXS3c7uo4lQr-sy1zwz3Q_6MfNaeKa0WV-zkA7S882tk_gDoknOyPjfZyJEL9IXxLvB702T0QIsYAYaVP0AApQU3yXz2oaRo" alt="editorial" />
                        <p class="v2-store-kicker">"Indie Feature · Editorial"</p>
                        <h3>"The Decentralized Renaissance: How Arcadestr is Changing Indie Distribution"</h3>
                        <p>
                            "The current landscape of distribution is dominated by centralized gatekeepers. Arcadestr introduces a direct creator-to-player model with protocol-native social discovery."
                        </p>
                        <div class="v2-social-actions">
                            <span>"bolt 1.2k"</span>
                            <span>"comment 84"</span>
                            <span>"repeat 210"</span>
                        </div>
                    </article>

                    <article class="v2-panel-glass v2-social-card">
                        <p class="v2-social-meta">"ProGamer_X · 2h ago"</p>
                        <p>
                            "Just reached Level 50 in Neon Abyss. Protocol leaderboard sync feels significantly faster this week."
                        </p>
                        <div class="v2-social-actions">
                            <span>"bolt 428"</span>
                            <span>"chat 12"</span>
                            <span>"share 5"</span>
                        </div>
                    </article>

                    <article class="v2-panel-glass v2-social-card">
                        <p class="v2-social-meta">"Void Studios · 5h ago"</p>
                        <p>
                            "Patch notes are live: improved cross-device save sync, fixed zap powerup bug, and added three community-designed levels."
                        </p>
                        <div class="v2-social-thumb-row">
                            <img src="https://lh3.googleusercontent.com/aida-public/AB6AXuBvkGgGjbsSrGCkp9nvRO185ObPhwb5Tm5BK6dU2pr_-aFmzhhhITjZzROpaxiBCdtGXmChuZlIP7Whz51IxfuvcfVYsi5FJMAttLedA2MGyqkIAjXPT0U-RgPe-xscK0uiJG6znKm9zl027DkQmjVk9NlUaXVXJX77CsliWK2GvdSq7L0UcOtAgIprwRUtDTq9pszRoV7zZGhqF7b5Je5asLvoImOQJygzsSZ8-Ejdx2qNJjeVzXsiDc8BY0uPzhqBhM9wjrVb70w" alt="patch visual" />
                            <img src="https://lh3.googleusercontent.com/aida-public/AB6AXuAZhuIm1gNvKcpdcVtaqIJGcokO7ISH_uVlUby0VCwf9cHZDyz0d5BOYAEOdAL1PYqaqLEQZBzjt821eNUaHItwmxoscXOYvCpD6AkVprUxb1033qj1SKOMIWR1Ww3KHBHAUMIjNPRd8CeTFO2Zu4q6fmxkwTHULj7tUkAzpI7L63R3idd_62s1tYN5pg89emHJEw_gKK03rS01TVdlHRot1LpLiR7Q6IflANfo7myr__BGFh1IxhFLNnaCOLj22eXis09aQ-_UyMU" alt="patch visual" />
                        </div>
                        <div class="v2-social-actions">
                            <span>"bolt 2.1k"</span>
                            <span>"chat 45"</span>
                            <span>"share 182"</span>
                        </div>
                    </article>
                </section>

                <aside class="v2-social-side">
                    <section class="v2-panel v2-social-side-card">
                        <h3>"Trending on Arcadestr"</h3>
                        <div class="v2-trending-list">
                            <div class="v2-trending-item"><strong>"#Gaming"</strong><p class="v2-social-meta">"12.4k notes this week"</p></div>
                            <div class="v2-trending-item"><strong>"#IndieDev"</strong><p class="v2-social-meta">"8.2k notes this week"</p></div>
                            <div class="v2-trending-item"><strong>"#Arcadestr"</strong><p class="v2-social-meta">"5.1k notes this week"</p></div>
                            <div class="v2-trending-item"><strong>"#Zaps"</strong><p class="v2-social-meta">"3.9k notes this week"</p></div>
                            <div class="v2-trending-item"><strong>"#Metaverse"</strong><p class="v2-social-meta">"2.2k notes this week"</p></div>
                        </div>
                        <button class="v2-btn-ghost">"Show more"</button>
                    </section>

                    <section class="v2-panel v2-social-side-card">
                        <h3>"Suggested for You"</h3>
                        <div class="v2-suggest-item">
                            <img src="https://lh3.googleusercontent.com/aida-public/AB6AXuCb78NJLyhZyIqoOo_a1xMVFZ93c0zfgqJ2deUV_rnyoot6pQha-o2Ttq1rfYP6Wcw6PT71pPAIizhD9NvWiqM8Yb8A9TmWiFCOqxdYw5sHzLVWH67JR7HwHxbO5B7p6uCIMPWXhgRPkwm8BPrJGfkA2RLhUW66Dao1HdAI4t5tGq6EAwT5zv-uf3pz5SUUekS9OGLRt7o3kikbXFUtQT1jis_atyUv2wm21dCw9x2zPAds7R_cSWk_FQDIseShMIV5fAOEiI_U_2E" alt="Cyber Pulse" />
                            <div><strong>"Cyber Pulse"</strong><p class="v2-social-meta">"Action · RPG"</p><p class="v2-social-meta">"@dex and 4 others play this"</p></div>
                        </div>
                        <div class="v2-suggest-item">
                            <img src="https://lh3.googleusercontent.com/aida-public/AB6AXuDfIq6ILaSrzJbJInGYexjOAJdOwOdUUyqmKc_rrDO7Wd3zwZzfg3CBudeKPpN-gOF4XmPqakja45oQdUf5IWFq7wyr_h6tttyS6nvI-vXw3eQxHRYfhXA4qvoYfGC9qn4NhiLLmDBWaylW6gJ1CZ9fW71iRcBp4FNYTMZPKBGIt20UMT78R3MvUShtgT8eP10xAl5C4yiuDEFS0nHSh97k33_SInB4YavQ4acN1fuO8Q0SGRpFghcu2L0JUeDl_Ru5iw_8vArr5HY" alt="Fragmented Mind" />
                            <div><strong>"Fragmented Mind"</strong><p class="v2-social-meta">"Puzzle · Indie"</p><p class="v2-social-meta">"@lynx plays this"</p></div>
                        </div>
                        <div class="v2-suggest-item">
                            <img src="https://lh3.googleusercontent.com/aida-public/AB6AXuBn25_cBuQByj83PmTDnT5O9Umfo6R0e8pcdJoo951y3DO_1rC_EFhAN__se3vuAahg4WhUwkI0E8npRAsa6CKXkOZa-o5socZ7l5YJGPrpGmmdPMjDU35_8XVgxOAKXJh54qyFZtFXGi-lTXTFlQu142c4EVBck6zylFUchXwYDWk-xXOcqZ3XThoz29GyXmhJ4lcu8nncF5M-y_9WozmGQDMWTrUTGUevlAKMregJZDQaeSg_ScPXBX4_IxB405t_gOMxrnqk2xA" alt="Arena Zero" />
                            <div><strong>"Arena Zero"</strong><p class="v2-social-meta">"Shooter · PvP"</p><p class="v2-social-meta">"Trending in your network"</p></div>
                        </div>
                        <button class="v2-btn-ghost">"Explore Social Catalog"</button>
                    </section>

                    <section class="v2-panel v2-social-side-card v2-zaps-card">
                        <h3>"Recent Zaps"</h3>
                        <div class="v2-zap-row"><span>"@satoshi zapped 1k"</span><strong>"SILICON DREAMS"</strong></div>
                        <div class="v2-zap-row"><span>"@hal zapped 500"</span><strong>"ARENA ZERO"</strong></div>
                    </section>
                </aside>
            </div>
        </section>
    }
}
