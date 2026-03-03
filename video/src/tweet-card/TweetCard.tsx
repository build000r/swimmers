import React from "react";
import { AbsoluteFill } from "remotion";
import { colors, fonts, branding, twitterLayouts } from "../theme";

/**
 * TweetCard — static frame for session-to-tweet diagrams.
 *
 * Renders as a Remotion <Still> at Twitter-optimal dimensions.
 * This is the "frame" that can later become an animated composition.
 *
 * Props match the session-to-tweet diagram JSON schema.
 */

export type TweetCardProps = {
  title: string;
  subtitle: string;
  stories: string[];
  flow: string[];
  skipped: string[];
  metric: string;
  layout?: keyof typeof twitterLayouts;
};

const defaults: TweetCardProps = {
  title: "Session Summary",
  subtitle: "What changed, why, and what we skipped",
  stories: ["Users can complete the core flow faster"],
  flow: ["Start", "Process", "Approve", "Done"],
  skipped: ["No skipped items recorded"],
  metric: "outcome pending",
  layout: "single",
};

export const TweetCard: React.FC<TweetCardProps> = (props) => {
  const p = { ...defaults, ...props };
  const layout = twitterLayouts[p.layout ?? "single"];

  return (
    <AbsoluteFill
      style={{
        background: `linear-gradient(135deg, ${colors.bgDark}, ${colors.bgMid})`,
        fontFamily: fonts.mono,
        padding: 32,
        display: "flex",
        flexDirection: "column",
        gap: 12,
      }}
    >
      {/* Header */}
      <div
        style={{
          background: colors.bgMid,
          border: `1px solid ${colors.orangeDark}`,
          borderRadius: 12,
          padding: "16px 20px",
        }}
      >
        <div
          style={{
            fontSize: layout.width > 800 ? 28 : 22,
            fontWeight: 700,
            color: colors.orangeLight,
          }}
        >
          {p.title}
        </div>
        <div
          style={{
            fontSize: layout.width > 800 ? 16 : 13,
            color: colors.textSecondary,
            marginTop: 4,
          }}
        >
          {p.subtitle}
        </div>
      </div>

      {/* Body: 3-column (single) or stacked */}
      <div
        style={{
          display: "flex",
          flex: 1,
          gap: 12,
          flexDirection: layout.width > 800 ? "row" : "column",
        }}
      >
        {/* Stories */}
        <Panel title="What Changed" borderColor={colors.orangeDark} flex={1}>
          {p.stories.map((s, i) => (
            <BulletItem key={i} color={colors.orange} text={s} />
          ))}
        </Panel>

        {/* Flow */}
        <Panel title="Flow" borderColor={colors.border} flex={1.1}>
          {p.flow.map((step, i) => (
            <FlowStep key={i} text={step} isLast={i === p.flow.length - 1} />
          ))}
        </Panel>

        {/* Skipped */}
        <Panel
          title="Skipped For Now"
          borderColor={colors.orangeDark}
          bg="#1a0f0a"
          flex={1.2}
        >
          {p.skipped.map((s, i) => (
            <BulletItem key={i} color={colors.orangeLight} text={s} />
          ))}
        </Panel>
      </div>

      {/* Footer */}
      <div
        style={{
          background: "#0b1220",
          border: `1px solid ${colors.orangeDark}`,
          borderRadius: 12,
          padding: "8px 16px",
          display: "flex",
          justifyContent: "space-between",
          alignItems: "center",
        }}
      >
        <div
          style={{
            fontSize: 15,
            fontWeight: 600,
            color: colors.orange,
          }}
        >
          {p.metric}
        </div>
        <div style={{ fontSize: 12, color: colors.textDim }}>
          {branding.tagline}
        </div>
      </div>
    </AbsoluteFill>
  );
};

// -- Sub-components --

const Panel: React.FC<{
  title: string;
  borderColor: string;
  bg?: string;
  flex?: number;
  children: React.ReactNode;
}> = ({ title, borderColor, bg = colors.bgMid, flex = 1, children }) => (
  <div
    style={{
      flex,
      background: bg,
      border: `1px solid ${borderColor}`,
      borderRadius: 12,
      padding: 16,
      display: "flex",
      flexDirection: "column",
      gap: 8,
      overflow: "hidden",
    }}
  >
    <div
      style={{
        fontSize: 16,
        fontWeight: 600,
        color: colors.textPrimary,
        marginBottom: 4,
      }}
    >
      {title}
    </div>
    {children}
  </div>
);

const BulletItem: React.FC<{ color: string; text: string }> = ({
  color,
  text,
}) => (
  <div style={{ display: "flex", alignItems: "flex-start", gap: 10 }}>
    <div
      style={{
        width: 8,
        height: 8,
        borderRadius: "50%",
        background: color,
        marginTop: 5,
        flexShrink: 0,
      }}
    />
    <div style={{ fontSize: 15, color: colors.textBody, lineHeight: 1.4 }}>
      {text}
    </div>
  </div>
);

const FlowStep: React.FC<{ text: string; isLast: boolean }> = ({
  text,
  isLast,
}) => (
  <div style={{ display: "flex", flexDirection: "column", alignItems: "center" }}>
    <div
      style={{
        background: colors.surfaceLight,
        border: `1px solid ${colors.borderLight}`,
        borderRadius: 8,
        padding: "8px 14px",
        fontSize: 14,
        color: colors.textSecondary,
        width: "100%",
        textAlign: "center",
      }}
    >
      {text}
    </div>
    {!isLast && (
      <div
        style={{
          width: 2,
          height: 10,
          background: colors.orange,
          margin: "2px 0",
        }}
      />
    )}
  </div>
);
