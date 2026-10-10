import type { CSSProperties, ReactNode } from "react";

// Icon geometry copied from the design files (16×16 grid unless noted).

interface IconProps {
  size?: number;
  className?: string;
  style?: CSSProperties;
  strokeWidth?: number;
}

function Svg(props: IconProps & { children: ReactNode; viewBox?: string; linecap?: boolean; linejoin?: boolean; defaultStroke?: number }) {
  const { size = 16, className, style, strokeWidth, children, viewBox = "0 0 16 16", linecap, linejoin, defaultStroke = 1.2 } = props;
  return (
    <svg
      width={size}
      height={size}
      viewBox={viewBox}
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth ?? defaultStroke}
      strokeLinecap={linecap ? "round" : undefined}
      strokeLinejoin={linejoin ? "round" : undefined}
      aria-hidden="true"
      className={className}
      style={style}
    >
      {children}
    </svg>
  );
}

const WARNING_PATHS = ["M8 2l6.3 11.5H1.7z", "M8 6.5v3.2M8 11.3v.4"];
const INFO_PATHS = ["M8 14.5a6.5 6.5 0 1 0 0-13 6.5 6.5 0 0 0 0 13z", "M8 7v4M8 4.8v.4"];
const ERROR_PATHS = ["M8 14.5a6.5 6.5 0 1 0 0-13 6.5 6.5 0 0 0 0 13z", "M5.9 5.9l4.2 4.2M10.1 5.9l-4.2 4.2"];

// The uBot mark from docs/brand/ubot-mark.svg (777×489, even-odd fill), drawn in currentColor.
export const UBOT_MARK_PATH =
  "M741.5 96.2 L734.2 85.5 L681.8 22.0 L667.0 10.0 L649.8 2.0 L637.8 0.2 L398.0 0.5 L323.0 64.8 L323.0 285.8 L319.5 300.2 " +
  "L314.5 307.8 L257.8 358.5 L246.0 367.2 L232.2 374.5 L216.0 380.2 L200.8 383.0 L186.5 383.5 L163.2 382.5 L152.8 379.8 " +
  "L143.5 375.5 L134.0 369.0 L127.2 362.0 L121.8 354.0 L117.5 343.5 L115.5 332.2 L115.5 162.5 L0.5 101.2 L0.2 339.2 " +
  "L3.2 363.2 L11.5 391.0 L23.5 415.0 L39.8 437.2 L53.8 451.2 L67.8 462.2 L79.5 469.8 L93.5 476.8 L119.0 485.5 " +
  "L143.8 489.5 L218.8 489.8 L238.2 487.8 L256.0 483.8 L272.0 478.5 L290.5 469.8 L311.8 456.5 L419.2 362.2 L428.8 349.5 " +
  "L433.0 338.0 L434.2 329.0 L435.0 291.5 L612.5 290.2 L620.2 292.2 L627.8 296.5 L673.2 342.2 L672.8 343.8 L627.8 383.8 " +
  "L435.8 384.2 L324.0 489.5 L649.2 489.8 L664.8 487.8 L678.0 483.5 L688.0 478.8 L702.0 469.2 L758.2 415.8 L764.5 407.2 " +
  "L770.8 394.8 L776.5 373.2 L776.8 326.5 L772.8 309.2 L767.5 297.0 L762.0 288.0 L752.5 276.8 L721.0 245.0 L702.0 228.5 " +
  "L735.8 193.5 L742.2 183.2 L747.5 171.0 L750.8 155.5 L750.8 125.8 L747.8 110.5 Z " +
  "M438.0 94.5 L607.5 93.8 L648.5 143.0 L606.2 183.2 L439.5 183.2 L438.0 182.2 Z";

export const LogoMark = ({ className }: { className?: string }) => (
  <svg viewBox="0 0 777 489" aria-hidden="true" focusable="false" className={className}>
    <path fill="currentColor" fillRule="evenodd" d={UBOT_MARK_PATH} />
  </svg>
);

export const RecipeIcon = (p: IconProps) => (
  <Svg {...p}><path d="M3.5 2.5h7a2 2 0 0 1 2 2v9h-7a2 2 0 0 1-2-2z" /><path d="M3.5 11.5a2 2 0 0 1 2-2h7" /></Svg>
);

export const RefreshIcon = (p: IconProps) => (
  <Svg {...p} linecap>
    <path d="M13 7.5a5 5 0 0 0-9.2-2.7" /><path d="M3.5 2v3h3" /><path d="M3 8.5a5 5 0 0 0 9.2 2.7" /><path d="M12.5 14v-3h-3" />
  </Svg>
);

export const PackageIcon = (p: IconProps) => (
  <Svg {...p} linejoin><path d="M8 1.8l5.5 3v6.4L8 14.2l-5.5-3V4.8z" /><path d="M2.5 4.8L8 7.8l5.5-3M8 7.8v6.4" /></Svg>
);

export const GraphIcon = (p: IconProps) => (
  <Svg {...p}>
    <rect x="1.5" y="2" width="4.5" height="3.5" rx="0.8" />
    <rect x="10" y="2" width="4.5" height="3.5" rx="0.8" />
    <rect x="5.75" y="10.5" width="4.5" height="3.5" rx="0.8" />
    <path d="M3.75 5.5v2.5h8.5V5.5M8 8v2.5" />
  </Svg>
);

export const WarningIcon = (p: IconProps) => (
  <Svg {...p} linejoin>{WARNING_PATHS.map((d) => <path key={d} d={d} />)}</Svg>
);

export const InfoIcon = (p: IconProps) => (
  <Svg {...p} linejoin>{INFO_PATHS.map((d) => <path key={d} d={d} />)}</Svg>
);

export const ErrorIcon = (p: IconProps) => (
  <Svg {...p} linejoin linecap>{ERROR_PATHS.map((d) => <path key={d} d={d} />)}</Svg>
);

export const GearIcon = (p: IconProps) => (
  <Svg {...p} viewBox="0 0 24 24" linecap linejoin defaultStroke={1.6}>
    <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" />
    <circle cx="12" cy="12" r="3" />
  </Svg>
);

export const SearchIcon = (p: IconProps) => (
  <Svg {...p} linecap defaultStroke={1.3}><circle cx="7" cy="7" r="4.5" /><path d="M10.5 10.5l3.5 3.5" /></Svg>
);

export const ChevronIcon = (p: IconProps) => (
  <Svg {...p} defaultStroke={1.5}><path d="M4 6l4 4 4-4" /></Svg>
);

export const ArrowUpIcon = (p: IconProps) => (
  <Svg {...p} linecap defaultStroke={1.4}><path d="M8 13V3M4 7l4-4 4 4" /></Svg>
);

export const ArrowDownIcon = (p: IconProps) => (
  <Svg {...p} linecap defaultStroke={1.5}><path d="M8 3v10M4 9l4 4 4-4" /></Svg>
);

export const TasksIcon = (p: IconProps) => (
  <Svg {...p} linecap defaultStroke={1.4}><path d="M5 4h8M5 8h8M5 12h8M2.5 4h.5M2.5 8h.5M2.5 12h.5" /></Svg>
);

export const CloseIcon = (p: IconProps) => (
  <Svg {...p} linecap defaultStroke={1.4}><path d="M4 4l8 8M12 4l-8 8" /></Svg>
);

export const CheckIcon = (p: IconProps) => (
  <Svg {...p} linecap linejoin defaultStroke={1.6}><path d="M3 8.5l3 3 7-7" /></Svg>
);

export const SpinnerIcon = (p: IconProps) => (
  <Svg {...p} linecap linejoin defaultStroke={1.6}><path d="M8 2.5a5.5 5.5 0 1 0 5.5 5.5" /></Svg>
);

export const PendingIcon = (p: IconProps) => (
  <Svg {...p} defaultStroke={1.3}><circle cx="8" cy="8" r="4.5" /></Svg>
);

export const ClockIcon = (p: IconProps) => (
  <Svg {...p} linecap linejoin defaultStroke={1.4}><circle cx="8" cy="8" r="5.5" /><path d="M8 5v3.2l2 1.3" /></Svg>
);

export const SkipIcon = (p: IconProps) => (
  <Svg {...p} linecap defaultStroke={1.6}><path d="M4.5 8h7" /></Svg>
);

export const StopIcon = ({ size = 14, className }: IconProps) => (
  <svg width={size} height={size} viewBox="0 0 16 16" aria-hidden="true" className={className}>
    <rect x="3.5" y="3.5" width="9" height="9" rx="1" fill="currentColor" />
  </svg>
);

export const RerunIcon = (p: IconProps) => (
  <Svg {...p} linecap defaultStroke={1.3}><path d="M13 8a5 5 0 1 1-1.5-3.6" /><path d="M12 1.5v3h-3" /></Svg>
);

export const FolderIcon = (p: IconProps) => (
  <Svg {...p} linejoin><path d="M1.5 4.5v8h13v-7H7.5L6 3.5H1.5z" /></Svg>
);

export const SmallInfoIcon = (p: IconProps) => (
  <Svg {...p} linecap linejoin defaultStroke={1.5}><path d="M8 7v4M8 4.5v.5" /></Svg>
);

// Window controls, drawn on a 10×10 grid with hairline strokes like the JetBrains Windows title bar.
export const MinimizeGlyph = () => (
  <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M0 5.5h10" stroke="currentColor" strokeWidth="1" /></svg>
);
export const MaximizeGlyph = () => (
  <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="currentColor" strokeWidth="1" /></svg>
);
export const RestoreGlyph = () => (
  <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1">
    <rect x="0.5" y="2.5" width="7" height="7" /><path d="M2.5 2.5V0.5h7v7h-2" />
  </svg>
);
export const CloseGlyph = () => (
  <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M0.5 0.5l9 9M9.5 0.5l-9 9" stroke="currentColor" strokeWidth="1" /></svg>
);
