import type { ComponentType } from "react"
import { Link } from "@tanstack/react-router"
import { cn } from "@repo/ui/lib/utils"

type NavItemProps = {
  to: "/ask"
  icon: ComponentType<{ className?: string }>
  label: string
}

export function NavItem({ to, icon: Icon, label }: NavItemProps) {
  return (
    <Link
      to={to}
      activeOptions={{ exact: false }}
      className={cn(
        "relative flex items-center gap-3 overflow-hidden rounded-md px-3 py-2 text-sm text-muted-foreground transition-colors",
        "hover:bg-accent/40 hover:text-foreground",
        "hover:[&_[data-nav-bar]]:scale-x-100"
      )}
      activeProps={{
        className:
          "bg-accent/60 font-medium text-foreground hover:bg-accent/60 [&_[data-nav-bar]]:scale-x-100",
      }}
    >
      <span
        data-nav-bar
        aria-hidden="true"
        className="absolute left-0 top-1/2 h-5 w-[2px] origin-left -translate-y-1/2 scale-x-0 rounded-r-full bg-[var(--corivo-amber)] transition-transform duration-200 ease-out"
        style={{
          boxShadow: "0 0 6px color-mix(in oklab, var(--corivo-amber) 60%, transparent)",
        }}
      />
      <Icon className="relative z-[1] h-4 w-4" />
      <span className="relative z-[1]">{label}</span>
    </Link>
  )
}
