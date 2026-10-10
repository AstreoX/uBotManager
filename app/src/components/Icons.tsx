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
