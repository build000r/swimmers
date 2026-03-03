import React from "react";
import { Folder, Still } from "remotion";
import { TweetCard } from "./tweet-card/TweetCard";
import { twitterLayouts } from "./theme";

/**
 * ThrongtermCompositions — all Remotion compositions for throngterm.
 *
 * Current:
 *   - TweetCard stills (single/dual/quad) for session-to-tweet images
 *
 * Future:
 *   - Animated session recaps
 *   - Thronglet character animations
 *   - Video changelogs
 */

const sampleProps = {
  title: "Session: Idempotency Fix",
  subtitle: "What changed, why it worked, and what we deferred",
  stories: [
    "Users can submit webhooks without duplicate invoice writes",
    "Users get more predictable retry behavior",
    "Users see fewer support issues around double charges",
  ],
  flow: [
    "Webhook received",
    "Idempotency key extracted",
    "Existing key lookup",
    "Write accepted or skipped",
    "Result logged",
  ],
  skipped: [
    "Redis-backed key cache: deferred to next pass",
    "CSV export of replay logs: not needed for v1",
  ],
  metric: "Duplicate write rate 3.2% → 0.1%",
};

export const ThrongtermCompositions: React.FC = () => {
  return (
    <Folder name="Throngterm">
      <Folder name="TweetCards">
        {/* Single image — 16:9, full-width in Twitter feed */}
        <Still
          id="TweetCard-Single"
          component={TweetCard}
          width={twitterLayouts.single.width}
          height={twitterLayouts.single.height}
          defaultProps={{ ...sampleProps, layout: "single" as const }}
        />

        {/* Dual card 1 — 7:8, left image in 2-image post */}
        <Still
          id="TweetCard-Dual"
          component={TweetCard}
          width={twitterLayouts.dual.width}
          height={twitterLayouts.dual.height}
          defaultProps={{ ...sampleProps, layout: "dual" as const }}
        />

        {/* Quad card — 2:1, for 4-image grid posts */}
        <Still
          id="TweetCard-Quad"
          component={TweetCard}
          width={twitterLayouts.quad.width}
          height={twitterLayouts.quad.height}
          defaultProps={{ ...sampleProps, layout: "quad" as const }}
        />
      </Folder>
    </Folder>
  );
};
