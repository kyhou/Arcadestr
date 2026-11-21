//! Mapping between a game listing and its Store Page.
//!
//! The merged Create/Edit game flow keeps one media list: the Store Page's.
//! The listing's `image` tags are derived from it at publication, and a listing
//! that has no Store Page yet gets a starting draft copied from its own fields.
//! Nothing here reaches the network; callers decide when a mapping is applied.

use std::collections::HashSet;

use crate::store_page::{StorePageDraft, StorePageMediaItem};
use crate::store_page_content_policy::{
    validate_store_page_url, MAX_GENRES, MAX_MEDIA_ITEMS, MAX_SCREENSHOTS, MAX_SUMMARY_CHARS,
    MAX_TITLE_CHARS,
};

/// The listing fields a Store Page can start from.
#[derive(Debug, Clone, Copy)]
pub struct ListingPresentation<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub tags: &'a [String],
    pub images: &'a [String],
}

/// What happened to each listing image when it was copied onto a Store Page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageImportReport {
    /// Media items added to the page (the cover can add two: capsule and hero).
    pub added: usize,
    /// Listing images already on the page; nothing was added for them.
    pub already_present: usize,
    /// Images the published page would reject (for example plain HTTP).
    pub invalid: Vec<String>,
    /// Valid images left out because the page reached a media limit.
    pub over_limit: Vec<String>,
}

impl ImageImportReport {
    pub fn skipped(&self) -> usize {
        self.invalid.len() + self.over_limit.len()
    }
}

/// A Store Page draft for `listing_coordinate`, filled from the listing's own
/// title, description, tags and images.
///
/// The cover (first image) becomes both the capsule and the hero so the page is
/// complete immediately; the remaining images become screenshots in order.
pub fn draft_from_listing(
    presentation_id: String,
    listing_coordinate: String,
    listing: ListingPresentation<'_>,
) -> (StorePageDraft, ImageImportReport) {
    let mut draft = StorePageDraft::new(presentation_id, vec![listing_coordinate]);
    let title = truncate_chars(listing.title.trim(), MAX_TITLE_CHARS);
    draft.content.basic.title = (!title.is_empty()).then_some(title);
    draft.content.basic.summary = summary_from_description(listing.description);
    draft.content.description_markdown = listing.description.trim().to_string();
    let genres = genres_from_tags(listing.tags);
    draft.content.discovery.genres = (!genres.is_empty()).then_some(genres);
    let report = import_listing_images(&mut draft, listing.images, listing.title);
    draft.sync_compact_tags();
    (draft, report)
}

/// Add the listing's images that are not already on the page. Additive only: an
/// existing capsule or hero is never replaced and nothing is removed.
pub fn import_listing_images(
    draft: &mut StorePageDraft,
    images: &[String],
    title: &str,
) -> ImageImportReport {
    let mut report = ImageImportReport::default();
    let label = if title.trim().is_empty() {
        "Game"
    } else {
        title.trim()
    };
    let mut present = draft
        .content
        .media
        .iter()
        .map(|item| item.url.clone())
        .collect::<HashSet<_>>();
    let mut screenshot_number = 0;
    for (index, url) in images.iter().enumerate() {
        let url = url.trim();
        if url.is_empty() {
            continue;
        }
        if validate_store_page_url(url).is_err() {
            report.invalid.push(url.to_string());
            continue;
        }
        if present.contains(url) {
            report.already_present += 1;
            continue;
        }
        let mut roles = Vec::new();
        if index == 0 {
            for role in ["capsule", "hero"] {
                if !has_role(draft, role) {
                    roles.push(role);
                }
            }
        }
        if roles.is_empty() {
            roles.push("screenshot");
        }
        for role in roles {
            if !has_capacity(draft, role) {
                report.over_limit.push(url.to_string());
                break;
            }
            let alt = match role {
                "capsule" | "hero" => format!("{label} cover art"),
                _ => {
                    screenshot_number += 1;
                    format!("{label} screenshot {screenshot_number}")
                }
            };
            let id = unique_media_id(draft, &format!("listing-{role}"));
            draft.content.media.push(StorePageMediaItem {
                id,
                media_type: "image".to_string(),
                role: role.to_string(),
                url: url.to_string(),
                sha256: None,
                mime_type: None,
                size: None,
                thumbnail_url: None,
                alt: Some(alt),
                caption: None,
                width: None,
                height: None,
            });
            report.added += 1;
        }
        present.insert(url.to_string());
    }
    report
}

/// The listing `image` tags implied by the Store Page media: the cover
/// (capsule, else hero) first, then screenshots in page order. Only images the
/// page itself would accept are included, and each URL appears once.
pub fn listing_images_from_media(media: &[StorePageMediaItem]) -> Vec<String> {
    let usable = |item: &&StorePageMediaItem| {
        item.media_type == "image" && validate_store_page_url(&item.url).is_ok()
    };
    let cover = media
        .iter()
        .filter(usable)
        .find(|item| item.role == "capsule")
        .or_else(|| media.iter().filter(usable).find(|item| item.role == "hero"));
    let mut seen = HashSet::new();
    cover
        .into_iter()
        .chain(
            media
                .iter()
                .filter(usable)
                .filter(|item| item.role == "screenshot"),
        )
        .filter(|item| seen.insert(item.url.clone()))
        .map(|item| item.url.clone())
        .collect()
}

/// First paragraph of a description, cut at a word boundary to fit the
/// Store Page summary limit.
pub fn summary_from_description(description: &str) -> Option<String> {
    let first = description
        .split("\n\n")
        .map(|paragraph| paragraph.split_whitespace().collect::<Vec<_>>().join(" "))
        .find(|paragraph| !paragraph.is_empty())?;
    if first.chars().count() <= MAX_SUMMARY_CHARS {
        return Some(first);
    }
    let budget = MAX_SUMMARY_CHARS - 1;
    let cut = truncate_chars(&first, budget);
    let at_word = cut
        .rfind(char::is_whitespace)
        .filter(|index| *index > budget / 2)
        .map_or(cut.as_str(), |index| &cut[..index]);
    Some(format!("{}…", at_word.trim_end()))
}

/// Align a Store Page edited together with its listing with the fields the
/// game form owns. The title, genres and summary always follow the listing
/// (the form edits them once, as the game title, tags and summary); the long
/// description starts from the listing description only while the page has
/// none of its own, so authored Store Page copy is never overwritten. Images
/// are not touched: the page media is the source the listing images are
/// derived from.
pub fn sync_with_listing(draft: &mut StorePageDraft, listing: ListingPresentation<'_>) {
    let title = truncate_chars(listing.title.trim(), MAX_TITLE_CHARS);
    draft.content.basic.title = (!title.is_empty()).then_some(title);
    let genres = genres_from_tags(listing.tags);
    draft.content.discovery.genres = (!genres.is_empty()).then_some(genres);
    draft.content.basic.summary = summary_from_description(listing.description);
    if draft.content.description_markdown.trim().is_empty() {
        draft.content.description_markdown = listing.description.trim().to_string();
    }
    draft.sync_compact_tags();
}

fn genres_from_tags(tags: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    tags.iter()
        .map(|tag| tag.trim())
        .filter(|tag| !tag.is_empty() && seen.insert(tag.to_lowercase()))
        .take(MAX_GENRES)
        .map(str::to_string)
        .collect()
}

fn has_role(draft: &StorePageDraft, role: &str) -> bool {
    draft.content.media.iter().any(|item| item.role == role)
}

fn has_capacity(draft: &StorePageDraft, role: &str) -> bool {
    let media = &draft.content.media;
    media.len() < MAX_MEDIA_ITEMS
        && (role != "screenshot"
            || media
                .iter()
                .filter(|item| item.role == "screenshot")
                .count()
                < MAX_SCREENSHOTS)
}

fn unique_media_id(draft: &StorePageDraft, prefix: &str) -> String {
    let existing = draft
        .content
        .media
        .iter()
        .map(|item| item.id.as_str())
        .collect::<HashSet<_>>();
    (1usize..)
        .map(|sequence| format!("{prefix}-{sequence}"))
        .find(|candidate| !existing.contains(candidate.as_str()))
        .unwrap_or_else(|| format!("{prefix}-new"))
}

fn truncate_chars(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn roles(draft: &StorePageDraft) -> Vec<(String, String)> {
        draft
            .content
            .media
            .iter()
            .map(|item| (item.role.clone(), item.url.clone()))
            .collect()
    }

    #[test]
    fn syncing_with_the_listing_keeps_authored_page_copy() {
        let mut draft = StorePageDraft::new("game".into(), vec!["30402:pub:game".into()]);
        draft.content.basic.title = Some("Old title".into());
        draft.content.discovery.genres = Some(tags(&["Puzzle"]));
        draft.content.description_markdown = "**Authored** page copy".into();
        let listing_tags = tags(&["Roguelike", " Strategy "]);
        sync_with_listing(
            &mut draft,
            ListingPresentation {
                title: " New title ",
                description: "Listing text.\n\nMore.",
                tags: &listing_tags,
                images: &[],
            },
        );
        assert_eq!(draft.content.basic.title.as_deref(), Some("New title"));
        assert_eq!(
            draft.content.discovery.genres,
            Some(tags(&["Roguelike", "Strategy"]))
        );
        // The summary follows the listing; the authored long description stays.
        assert_eq!(
            draft.content.basic.summary.as_deref(),
            Some("Listing text.")
        );
        assert_eq!(draft.content.description_markdown, "**Authored** page copy");
        assert_eq!(draft.compact_tags.title.as_deref(), Some("New title"));

        // An emptied listing title clears the page title instead of keeping a stale one.
        sync_with_listing(
            &mut draft,
            ListingPresentation {
                title: "  ",
                description: "",
                tags: &[],
                images: &[],
            },
        );
        assert_eq!(draft.content.basic.title, None);
        assert_eq!(draft.content.discovery.genres, None);
        assert_eq!(draft.content.basic.summary, None);
        assert_eq!(draft.content.description_markdown, "**Authored** page copy");
    }

    #[test]
    fn a_new_page_starts_from_the_listing() {
        let images = tags(&[
            "https://cdn.example/cover.png",
            "https://cdn.example/one.png",
            "https://cdn.example/two.png",
        ]);
        let listing_tags = tags(&["Roguelike", "roguelike", " Strategy ", ""]);
        let (draft, report) = draft_from_listing(
            "game".into(),
            "30402:pub:game".into(),
            ListingPresentation {
                title: "Cogmind",
                description: "Build a robot\nfrom parts.\n\nSecond paragraph.",
                tags: &listing_tags,
                images: &images,
            },
        );

        assert_eq!(draft.listing_coordinates, ["30402:pub:game"]);
        assert_eq!(draft.content.basic.title.as_deref(), Some("Cogmind"));
        assert_eq!(
            draft.content.basic.summary.as_deref(),
            Some("Build a robot from parts.")
        );
        assert!(draft
            .content
            .description_markdown
            .ends_with("Second paragraph."));
        assert_eq!(
            draft.content.discovery.genres,
            Some(tags(&["Roguelike", "Strategy"]))
        );
        assert_eq!(
            roles(&draft),
            vec![
                ("capsule".into(), "https://cdn.example/cover.png".into()),
                ("hero".into(), "https://cdn.example/cover.png".into()),
                ("screenshot".into(), "https://cdn.example/one.png".into()),
                ("screenshot".into(), "https://cdn.example/two.png".into()),
            ]
        );
        assert_eq!(report.added, 4);
        assert_eq!(draft.compact_tags.title.as_deref(), Some("Cogmind"));
        assert_eq!(
            draft.content.media[2].alt.as_deref(),
            Some("Cogmind screenshot 1")
        );
    }

    #[test]
    fn images_the_page_would_reject_are_reported_not_copied() {
        let images = tags(&["http://insecure.example/cover.png", "not a url"]);
        let mut draft = StorePageDraft::new("game".into(), vec![]);
        let report = import_listing_images(&mut draft, &images, "Game");
        assert!(draft.content.media.is_empty());
        assert_eq!(report.invalid.len(), 2);
        assert_eq!(report.skipped(), 2);
    }

    #[test]
    fn importing_is_additive_and_never_duplicates() {
        let images = tags(&[
            "https://cdn.example/cover.png",
            "https://cdn.example/one.png",
        ]);
        let mut draft = StorePageDraft::new("game".into(), vec![]);
        draft.content.media.push(StorePageMediaItem {
            id: "my-hero".into(),
            media_type: "image".into(),
            role: "hero".into(),
            url: "https://cdn.example/wide-hero.png".into(),
            sha256: None,
            mime_type: None,
            size: None,
            thumbnail_url: None,
            alt: None,
            caption: None,
            width: None,
            height: None,
        });

        let first = import_listing_images(&mut draft, &images, "Game");
        // The publisher's own hero is kept; the cover only fills the empty capsule.
        assert_eq!(first.added, 2);
        assert_eq!(
            roles(&draft),
            vec![
                ("hero".into(), "https://cdn.example/wide-hero.png".into()),
                ("capsule".into(), "https://cdn.example/cover.png".into()),
                ("screenshot".into(), "https://cdn.example/one.png".into()),
            ]
        );

        let again = import_listing_images(&mut draft, &images, "Game");
        assert_eq!(again.added, 0);
        assert_eq!(again.already_present, 2);
        assert_eq!(draft.content.media.len(), 3);
    }

    #[test]
    fn screenshots_beyond_the_page_limit_are_reported() {
        let images = (0..=MAX_SCREENSHOTS + 1)
            .map(|index| format!("https://cdn.example/{index}.png"))
            .collect::<Vec<_>>();
        let mut draft = StorePageDraft::new("game".into(), vec![]);
        let report = import_listing_images(&mut draft, &images, "Game");
        let screenshots = draft
            .content
            .media
            .iter()
            .filter(|item| item.role == "screenshot")
            .count();
        assert_eq!(screenshots, MAX_SCREENSHOTS);
        assert_eq!(report.over_limit.len(), 1);
    }

    #[test]
    fn listing_images_are_derived_cover_first_without_duplicates() {
        let mut draft = StorePageDraft::new("game".into(), vec![]);
        import_listing_images(
            &mut draft,
            &tags(&[
                "https://cdn.example/cover.png",
                "https://cdn.example/one.png",
                "https://cdn.example/two.png",
            ]),
            "Game",
        );
        assert_eq!(
            listing_images_from_media(&draft.content.media),
            tags(&[
                "https://cdn.example/cover.png",
                "https://cdn.example/one.png",
                "https://cdn.example/two.png",
            ])
        );

        // Without a capsule the hero is the cover; videos and invalid URLs never
        // become listing image tags.
        draft.content.media.retain(|item| item.role != "capsule");
        draft.content.media[0].url = "https://cdn.example/hero.png".into();
        draft.content.media.push(StorePageMediaItem {
            media_type: "video".into(),
            role: "trailer".into(),
            url: "https://cdn.example/trailer.webm".into(),
            ..draft.content.media[1].clone()
        });
        draft.content.media[2].url = "not a url".into();
        assert_eq!(
            listing_images_from_media(&draft.content.media),
            tags(&[
                "https://cdn.example/hero.png",
                "https://cdn.example/one.png"
            ])
        );
    }

    #[test]
    fn long_descriptions_become_a_summary_cut_at_a_word() {
        let long = "word ".repeat(200);
        let summary = summary_from_description(&long).expect("summary");
        assert!(summary.chars().count() <= MAX_SUMMARY_CHARS);
        assert!(summary.ends_with("word…"));
        assert_eq!(summary_from_description("  \n\n  "), None);
    }
}
