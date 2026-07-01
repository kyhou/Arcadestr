# Marketplace Empty State Banner Design

## Goal

When no marketplace products are available, the Store front and Browse Games pages must show a clear empty-state banner instead of leaving the content area blank.

## Scope

- Store front `Trending Games` section.
- Browse Games listing area, including filtered results.
- No backend or marketplace fetching behavior changes.

## Store Front Behavior

When loading has finished, there is no fetch error, and `listings` is empty, render an inline banner in the `Trending Games` area.

Copy:

- Title: `No products found`
- Body: `We could not find any marketplace listings from the connected relays. Try again later or check your relay connection.`

## Browse Games Behavior

When loading has finished and the displayed, filtered product list is empty, render an inline banner in the browse content area.

Copy:

- Title: `No products found`
- Body: `No games match the current marketplace results or filters.`

## Styling

Use existing UI tokens and utility classes:

- `bg-surface-container-high`
- `border border-outline-variant/15`
- `rounded-xl`
- `text-on-surface`
- `text-on-surface-variant`

Avoid new CSS unless needed to keep duplication low.

## Testing

- Prefer existing testable helper functions for Browse Games if a helper already controls empty-state visibility.
- Run `rtk cargo check -p arcadestr-app` after implementation.
