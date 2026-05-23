// Corivo "Momo" mascot — procedurally drawn SVG blob with friendly face.
// Mirror of apps/web/components/ui/CorivoBlobSvg.tsx so the desktop and the
// marketing site can grow toward a single shared component without the web
// app needing to take a workspace dep on @repo/ui yet.
type BlobColor = "brand" | "amber" | "yellow" | "orange" | "blue"

// "brand" mirrors the official logo (system-v0 §07): body #EF7551, face
// features #352010. Kept identical across light + dark themes — the
// mascot is a brand asset, not a UI signal.
const colorMap: Record<BlobColor, { fill: string; face: string }> = {
  brand: { fill: "#EF7551", face: "#352010" },
  amber: { fill: "#F5A07A", face: "rgba(80,30,10,0.80)" },
  yellow: { fill: "#f5c842", face: "rgba(40,30,0,0.75)" },
  orange: { fill: "#f5874a", face: "rgba(40,15,0,0.75)" },
  blue: { fill: "#42b4f5", face: "rgba(0,20,40,0.75)" },
}

interface CorivoBlobProps {
  color?: BlobColor
  size?: number
  className?: string
  noFace?: boolean
  expression?: "smile" | "cool" | "sleepy"
}

export function CorivoBlob({
  color = "amber",
  size = 80,
  className,
  noFace = false,
  expression = "smile",
}: CorivoBlobProps) {
  const { fill, face } = colorMap[color]

  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 100 100"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={className}
    >
      {/* Off-balance bean body — right side wider, smooth rounded top */}
      <path
        d="M48 5
           C66 2,87 15,87 36
           C88 57,78 81,56 90
           C42 96,18 91,11 75
           C4 60,7 41,16 31
           C26 15,38 7,48 5Z"
        fill={fill}
      />

      {!noFace && expression === "smile" && (
        <>
          <ellipse cx="35" cy="44" rx="5" ry="6" fill={face} />
          <ellipse cx="61" cy="41" rx="5" ry="6" fill={face} />
          <circle cx="37" cy="41" r="1.8" fill="rgba(255,255,255,0.85)" />
          <circle cx="63" cy="38" r="1.8" fill="rgba(255,255,255,0.85)" />
          <path
            d="M 31 58 Q 48 71 64 58"
            stroke={face}
            strokeWidth="3.5"
            strokeLinecap="round"
            fill="none"
          />
        </>
      )}

      {!noFace && expression === "cool" && (
        <>
          <path d="M 29 43 Q 35 40 41 43" stroke={face} strokeWidth="3" strokeLinecap="round" fill="none" />
          <path d="M 55 40 Q 61 37 67 40" stroke={face} strokeWidth="3" strokeLinecap="round" fill="none" />
          <path d="M 33 60 Q 48 68 62 57" stroke={face} strokeWidth="3.5" strokeLinecap="round" fill="none" />
        </>
      )}

      {!noFace && expression === "sleepy" && (
        <>
          <ellipse cx="35" cy="46" rx="5" ry="3.5" fill={face} />
          <ellipse cx="61" cy="43" rx="5" ry="3.5" fill={face} />
          <path d="M 33 59 Q 48 66 62 59" stroke={face} strokeWidth="3" strokeLinecap="round" fill="none" />
        </>
      )}
    </svg>
  )
}

export function BlobBrand(props: Omit<CorivoBlobProps, "color">) {
  return <CorivoBlob {...props} color="brand" />
}
export function BlobAmber(props: Omit<CorivoBlobProps, "color">) {
  return <CorivoBlob {...props} color="amber" />
}
export function BlobYellow(props: Omit<CorivoBlobProps, "color">) {
  return <CorivoBlob {...props} color="yellow" expression="cool" />
}
export function BlobOrange(props: Omit<CorivoBlobProps, "color">) {
  return <CorivoBlob {...props} color="orange" expression="sleepy" />
}
export function BlobBlue(props: Omit<CorivoBlobProps, "color">) {
  return <CorivoBlob {...props} color="blue" />
}
