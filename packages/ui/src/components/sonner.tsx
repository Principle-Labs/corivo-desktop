import { Toaster as Sonner } from "sonner"

type ToasterProps = React.ComponentProps<typeof Sonner>

/**
 * Corivo-flavored sonner Toaster. Differences from the shadcn default:
 *
 * 1. **Theme follows the app.** Sonner's `theme` prop is "system" so its
 *    own default treatments adapt to the OS appearance, matching what
 *    `useThemeSync` sets on `<html data-theme>`. Hard-coding "light" the
 *    way the shadcn template did made every toast look like a white
 *    sticker dropped onto a dark app.
 *
 * 2. **Visual tokens align with the rest of the chrome.** Card uses the
 *    same `rounded-lg` + `border-border/40` + `shadow-md` that workflow
 *    list rows and chat threads use, so a toast reads as "Corivo
 *    component" not "browser notification".
 *
 * 3. **Title uses `font-display`** (the brand display face) and the
 *    description settles into the `12px / muted-foreground` typography
 *    pair used everywhere else.
 *
 * 4. **Cancel = ghost.** The dismiss affordance ("收起" on loading
 *    toasts) is a borderless text button instead of the chunky grey
 *    pill `bg-muted` produced. `!important` overrides are required
 *    because sonner ships baked-in pill styles on the cancel/action
 *    slots that we'd otherwise inherit.
 *
 * Explicit `closeButton` is intentionally NOT set: long-running loading
 * toasts already get an explicit `cancel: { label: "收起" }` button —
 * having both an X and a "收起" was two affordances for one action.
 */
const Toaster = ({ ...props }: ToasterProps) => {
  return (
    <Sonner
      theme="system"
      className="toaster group"
      toastOptions={{
        classNames: {
          toast: [
            "group toast",
            "group-[.toaster]:bg-background",
            "group-[.toaster]:text-foreground",
            "group-[.toaster]:border",
            "group-[.toaster]:border-border/40",
            "group-[.toaster]:rounded-lg",
            "group-[.toaster]:shadow-md",
            // Pull the default sonner padding in a touch — workflow
            // list rows and dialogs use 16 px / 12 px paddings, so
            // a 14 px toast pad reads as the same family.
            "group-[.toaster]:!p-3.5",
          ].join(" "),
          title: [
            "group-[.toast]:font-display",
            "group-[.toast]:text-[13.5px]",
            "group-[.toast]:font-medium",
            "group-[.toast]:tracking-[-0.005em]",
            "group-[.toast]:text-foreground",
          ].join(" "),
          description: [
            "group-[.toast]:text-[12px]",
            "group-[.toast]:text-muted-foreground",
            "group-[.toast]:leading-[1.5]",
          ].join(" "),
          // Default sonner icon (success / error / info) is
          // visually loud and doesn't match the icon-light vibe of
          // workflow rows / dialogs. Shrink + dim so it reads as
          // accent, not chrome. Loading variant's spinner inherits
          // the same selector and stays readable at this size.
          icon: [
            "group-[.toast]:!size-3.5",
            "group-[.toast]:opacity-60",
            "group-[.toast]:mt-0.5",
          ].join(" "),
          // Both action and cancel render as text-only "links" with
          // a faint hover bg — Corivo button system has no chunky
          // pills in passive surfaces, and the toast is one of the
          // most passive surfaces we have. Differentiation is purely
          // color weight: action uses foreground, cancel uses
          // muted-foreground.
          actionButton: [
            "!bg-transparent",
            "!text-foreground",
            "hover:!bg-muted/60",
            "!h-auto",
            "!px-2",
            "!py-1",
            "!text-[11.5px]",
            "!font-medium",
            "!rounded-md",
            "!shadow-none",
            "!border-0",
            // Sonner's stylesheet applies `margin-left: auto` to
            // EVERY [data-button] — so when both a cancel and an
            // action button are present, each one is independently
            // pushed to the right and a stray flex gap opens up
            // between them (visible as "收起 ……… 取消" in the
            // workflow loading toast: 收起 sits in the middle, 取消
            // at the far right, neither feels grouped). When this
            // button is NOT the first of its type (i.e. there's a
            // cancel sibling before it), pin its margin-left to a
            // small fixed value so the pair reads as one button
            // group at the right edge.
            "[&:not(:first-of-type)]:!ml-1.5",
          ].join(" "),
          cancelButton: [
            "!bg-transparent",
            "!text-muted-foreground",
            "hover:!text-foreground",
            "hover:!bg-muted/60",
            "!h-auto",
            "!px-2",
            "!py-1",
            "!text-[11.5px]",
            "!font-medium",
            "!rounded-md",
            "!shadow-none",
            "!border-0",
          ].join(" "),
        },
      }}
      {...props}
    />
  )
}

export { Toaster }
