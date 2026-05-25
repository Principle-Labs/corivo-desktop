// English UI dictionary. Type-checked against `zh.ts` via the
// `satisfies LocaleDict` assertion so missing/extra keys fail compile.

import type { LocaleDict } from "./zh";

export const en: LocaleDict = {
  common: {
    loading: "Loading…",
    saving: "Saving…",
    cancel: "Cancel",
    confirm: "Confirm",
    delete: "Delete",
    remove: "Remove",
    add: "Add",
    save: "Save",
    test: "Test",
    enable: "Enable",
    disable: "Disable",
    refresh: "Rescan",
    selectAll: "Select all",
    clearAll: "Clear",
    unknown: "Unknown",
    empty: "Empty",
    saveFailed: (detail: string) => `Save failed: ${detail}`,
    loginFailed: (detail: string) => `Sign-in failed: ${detail}`,
    deleteFailed: (detail: string) => `Delete failed: ${detail}`,
    addFailed: (detail: string) => `Add failed: ${detail}`,
    clearFailed: (detail: string) => `Clear failed: ${detail}`,
    connectFailed: (detail: string) => `Connection failed: ${detail}`,
    deleteOpFailed: (detail: string) => `Wipe failed: ${detail}`,
    setupFailed: (detail: string) => `Update failed: ${detail}`,
    resetFailed: (detail: string) => `Reset failed: ${detail}`,
    logoutFailed: (detail: string) => `Sign-out failed: ${detail}`,
  },

  app: {
    booting: "Starting Corivo…",
    forceUpdate: {
      title: "Updating Corivo",
      errorTitle: "Update failed",
      errorFallback: "Something went wrong while installing the new version.",
      preparing: "Preparing the new version.",
      foundVersion: (version: string) =>
        `Found version ${version}, will install and restart.`,
      downloadProgress: (downloaded: string, total: string) =>
        `Downloaded ${downloaded} / ${total}`,
      retry: "Retry update",
    },
  },

  nav: {
    ask: "Ask",
  },

  sidebar: {
    workflows: "Routines",
    quickAskHintPrefix: "Double-tap",
    quickAskHintSuffix: "to summon Corivo",
    quickAskHintTitle: "Double-tap the hotkey inside any app to summon Corivo",
    balanceTitle: "View balance & top up",
    balanceAction: "TOP UP",
  },

  status: {
    capturing: "Context enabled",
    stopped: "Paused",
    disabled: "Context off",
    pausedFor: (remaining: string) => `Paused · ${remaining} left`,
    pending: "Working…",
    start: "Resume",
    stop: "Pause",
    intervalFormat: (seconds: number) => `every ${seconds}s`,
  },

  contextPopover: {
    triggerAriaLabel: "Open data & privacy menu",
    header: "Data and Privacy",
    intro: {
      title: "Context awareness",
      body: "Corivo remembers your work across apps. No integrations needed.",
      learnMore: "Learn more",
      dismissAria: "Don't show again",
    },
    pause: {
      label: "Pause context awareness",
      options: {
        fiveMin: "5 minutes",
        fifteenMin: "15 minutes",
        thirtyMin: "30 minutes",
        oneHour: "1 hour",
        oneDay: "1 day",
      },
      toast: (label: string) => `Paused for ${label}`,
      toastFailed: (detail: string) => `Pause failed: ${detail}`,
    },
    resume: {
      label: "Resume now",
      toast: "Resumed",
      toastFailed: (detail: string) => `Resume failed: ${detail}`,
    },
    delete: {
      label: "Delete data",
      options: {
        lastFiveMin: "Last 5 minutes",
        lastFifteenMin: "Last 15 minutes",
        custom: "Custom…",
      },
      toast: (frames: number) =>
        `Deleted ${frames} ${frames === 1 ? "entry" : "entries"}`,
      toastFailed: (detail: string) => `Delete failed: ${detail}`,
      confirm: {
        title: "Delete data",
        description:
          "Choose a duration to remove data for. Data collected during the selected period will be permanently deleted.",
        warning: "Warning: this action is not reversible. Please be certain.",
        durationLabel: "Enter the duration for which you'd like to delete your data",
        durationPlaceholder: "Enter duration",
        unitMinutes: "Minutes",
        unitHours: "Hours",
        unitDays: "Days",
        confirmTypeLabel: "To verify, type 'delete' below",
        confirmPlaceholder: "Enter 'delete' to confirm",
        cancel: "Cancel",
        confirm: "Delete data",
        confirming: "Deleting…",
      },
    },
    exclusions: {
      label: "Excluded apps and websites",
      manage: "Manage",
    },
    statusRow: {
      running: "Context enabled",
      paused: "Paused",
      stopped: "Context off",
    },
  },

  user: {
    defaultName: "Corivo user",
    comingSoon: "Coming soon",
    subscription: "Subscription",
    settings: "Settings",
    logout: "Sign out",
    logoutSuccess: "Signed out",
    logoutConfirm: {
      title: "Sign out of Corivo?",
      description:
        "Your local tasks and context stay put, but you'll need to sign in again to keep using Corivo.",
      confirm: "Sign out",
      pending: "Signing out…",
    },
  },

  billing: {
    title: "Add credits",
    description:
      "Credits are deducted by actual token usage. Your balance never expires.",
    balance: {
      label: "Current balance",
      refresh: "Refresh balance",
      lastPaymentPrefix: (amount: string) => `Last top-up $${amount}`,
      status: {
        credited: "Credited",
        paid: "Paid",
        pending: "Processing",
        failed: "Failed",
      },
    },
    picker: {
      label: "Choose an amount",
      hint: "Pick a tier — durations are rough estimates",
      recommendedBadge: "Recommended",
      tiers: {
        starter: { name: "Starter", duration: "About a week of light use" },
        standard: { name: "Standard", duration: "About a month of daily use" },
        longTerm: {
          name: "Long-term",
          duration: "About a quarter of steady use",
        },
      },
      cta: {
        idle: "Choose an amount",
        ready: (amount: number) => `Pay $${amount} via Stripe`,
        submitting: "Opening Stripe…",
      },
    },
    usage: {
      label: "Credits power",
      items: {
        ask: {
          label: "/ask deep conversations",
          hint: "Includes tool calls and screen recall",
        },
        quickAsk: {
          label: "Quick Ask instant answers",
          hint: "Lightweight Q&A from the global hotkey",
        },
        index: {
          label: "Screen history indexing & summaries",
          hint: "Quietly turns your work into searchable memory in the background",
        },
      },
    },
    trust: {
      primary: "Secured by Stripe · Billed in USD · Card details never stored",
      secondary:
        "Visa · Mastercard · American Express accepted. Receipt sent to your sign-in email.",
    },
    toasts: {
      openedBrowser: "Stripe checkout opened in your browser",
      openedBrowserHint:
        "After paying, come back to Corivo and tap the refresh button.",
      loadFailed: "Couldn't load balance",
      checkoutFailed: "Couldn't start checkout",
      refreshFailed: "Refresh failed",
    },
  },

  frame: {
    drawerEyebrow: "Screen context",
    loading: "Loading…",
    notFound: "Context not found.",
    unknownApp: "Unknown app",
    emptyText: "No readable text was extracted from this context entry.",
    section: {
      text: "Text",
      data: "Data",
    },
  },

  ask: {
    threadList: {
      title: "Tasks",
      newThread: "Start with Corivo",
      empty: "No tasks yet. Hit \"Start with Corivo\" above to begin.",
      recent: "Recent",
      pinned: "Pinned",
      archived: "Archived",
      archivedCount: (count: number) => `Archived (${count})`,
      searchPlaceholder: "Search tasks",
      searchEmpty: (query: string) =>
        `No tasks matching "${query}"`,
      action: {
        menu: "More actions",
        pin: "Pin",
        unpin: "Unpin",
        archive: "Archive",
        unarchive: "Restore",
        delete: "Delete",
      },
      activity: {
        running: "Running",
        unread: "New reply",
        error: "Run failed",
      },
      workflow: {
        prefixIcon: "⏰",
        statusSuccess: "Success",
        statusFailure: "Failed",
        bannerTitle: (name: string) => `Routine: ${name}`,
        bannerSubtitle: (when: string) => `Ran ${when} · read-only view`,
        bannerManageAction: "Manage routine",
        readOnlyHint:
          "Routine runs are read-only. Edit or re-run from the routines page.",
      },
    },
    placeholderTitle: "New task",
    composer: {
      placeholder: "Keep this task moving…",
      streamingPlaceholder: "Responding, hold on…",
      sending: "Working",
      send: "Hand off to Corivo",
      stop: "Pause",
      stopHint: "Pause this task",
    },
    empty: {
      title: "No tasks started yet.",
      typeHint: "Type a task below, or —",
      hotkeyHint: "double-tap inside any app, I'll surface in a beat",
    },
    welcome:
      "Tell me a task below. I'll search your frames and cite what I find.",
    noActiveThread: "No task selected",
    streamErrorFallback: "stream finished with error",
    permission: {
      title: "Agent is requesting permission",
      action: "Action:",
      reason: "Reason:",
      viewDetails: "View arguments",
      hint: "Once allowed, Corivo will run this. For destructive or irreversible actions, expand the arguments to double-check.",
      allow: "Allow",
      deny: "Deny",
    },
  },

  chat: {
    thinking: "Thinking",
    thoughtProcess: "Thought process",
    emptyReply: "(empty reply)",
    inProgress: "In progress…",
    frameAnchorTitle: "Window the message was anchored to",
    frameAnchorOne: "Anchored to a window",
    frameAnchorN: (count: number) => `Anchored to ${count} windows`,
    youLabel: "You",
    corivoLabel: "Corivo",
    streaming: "Streaming",
    completed: "Done",
    citationsLabel: "Context",
    citationAria: (index: number) => `Context ${index}`,
    copy: "Copy",
    copied: "Copied",
    toolVerbs: {
      recallFrames: "Searching frames",
      runCommand: "Running command",
      readFile: "Reading file",
      editFile: "Editing file",
      writeFile: "Writing file",
      grep: "Searching code",
      glob: "Finding files",
      webFetch: "Fetching web page",
      webSearch: "Web search",
      todo: "Updating plan",
      timesSuffix: (n: number) => ` (×${n})`,
      genericCall: (name: string) => `Calling ${name}`,
    },
  },

  quickAsk: {
    title: "Quick Ask",
    titleWithApp: (app: string) => `Quick Ask · ${app}`,
    closeAria: "Close",
    closeTitle: "Close (Esc)",
    historyAria: "Task history",
    historyTitle: "Switch to a previous task",
    newAria: "New task",
    newTitle: "New task",
    inputPlaceholder: "Ask anything…",
    openInAppAria: "Open in app",
    openInAppTitle: "Open this task in the app",
    micAria: "Voice input (unavailable)",
    micTitle: "Voice input (unavailable)",
    sendAria: "Send",
    answeringAria: "Answering",
    sendTitle: "Send (Enter)",
    answeringTitle: "Answering…",
    stopAria: "Stop generating",
    stopTitle: "Stop generating",
    picker: {
      newSession: "New task",
      loading: "Loading…",
      empty: "No previous tasks",
      untitled: "Untitled task",
    },
    relativeTime: {
      justNow: "Just now",
      minutesAgo: (n: number) => `${n} min ago`,
      hoursAgo: (n: number) => `${n} h ago`,
      daysAgo: (n: number) => `${n} d ago`,
    },
    focus: {
      currentApp: "Current app",
      currentWindow: "Current window",
      readingWindow: "Reading window…",
      excludedSuffix: "excluded",
      cantReadWindow: "Can't read window contents",
      switchTo: (app: string) => `+ Switch to ${app}`,
      switchTitle: "Anchor the next message to the current frontmost app",
    },
    titlebarHint: "Esc",
    resizeAria: "Resize window",
    resizeTitle: "Drag to resize",
  },

  onboarding: {
    steps: {
      permission: "Permissions",
      demo: "Try it",
      shortcut: "Shortcut",
    },
    nav: {
      back: "Back",
      continue: "Continue",
    },
    permission: {
      stepLabel: "01 · Permissions",
      title: "Grant access to Corivo",
      subtitle:
        "Corivo needs two macOS permissions to understand your context.",
      ax: {
        name: "Accessibility",
        desc: "Used to read text on your screen",
      },
      screen: {
        name: "Screen Recording",
        desc: "Used to recognize text on your screen",
      },
      status: {
        pending: "Pending",
        asking: "Asking…",
        granted: "Granted",
      },
      grant: "Grant",
      foot: "Continues automatically once both granted\nStored as structured text on your Mac",
      denied:
        "macOS won't pop the dialog again. Open System Settings → Privacy & Security, find Corivo, and toggle it on. Corivo will auto-detect.",
      openSettings: "Open System Settings",
    },
    demo: {
      stepLabel: "02 · Try it",
      title: "Here's how Corivo works",
      subtitle:
        "Below is a sample doc — Corivo's already seen it. Just ask.",
      url: "Demo · 2026 Q3 Product Plan",
      doc: {
        title: "2026 Q3 Product Plan",
        meta: "Alex Chen · Product · 2026-03-15",
        bgH: "Background",
        bgP: "Last quarter active users grew 32%, but week-one retention sits at just 41% — well below industry baseline. The core issues are an over-long onboarding and unclear value framing.",
        goalH: "Goals",
        goalP:
          "Lift week-one retention to 55%; raise onboarding completion from 68% to 80%.",
        planH: "Plan",
        plan1: "Rebuild onboarding around three focused steps",
        plan2:
          "Step 1 focuses on permissions with a clear privacy commitment",
        plan3: "Step 2 demonstrates the product in a real scenario",
        plan4: "Step 3 introduces the keyboard shortcut",
        msH: "Milestones",
        ms1: "Jul 15 — design done & reviewed",
        ms2: "Aug 1 — dev complete, internal beta",
        ms3: "Aug 20 — full rollout",
        ms4: "End of Sep — review, plan next iteration",
        riskH: "Risks",
        riskP:
          "Permissions are a sensitive moment — needs an A/B on two tone strategies.",
      },
      askPlaceholder: "Ask about what's on screen…",
      chip: "Summarize what I'm looking at, save to Apple Notes",
      cta: "Continue",
      skip: "Skip",
      askTitle: "Try Corivo",
      send: "Send",
      stop: "Stop",
      retry: "Retry",
      thinking: "Thinking…",
      you: "You",
      corivoLabel: "Corivo",
      errorLabel: "Something went wrong",
      networkErrorHint: "Network issue or service unavailable. Check your connection and try again.",
    },
    shortcut: {
      stepLabel: "03 · Shortcut",
      title: "Double-tap the hotkey to summon Corivo",
      subtitle:
        "Use your computer as you normally would — double-tap the system hotkey to bring up Corivo whenever you need it.",
      cue: "Tap twice",
      cta: "Enter Corivo",
    },
  },

  login: {
    brand: "Corivo",
    tagline: "a shadow agent that actually fits how you work",
    eyebrow: "Sign in",
    title: "Sign in to Corivo.",
    googleSignIn: "Continue with Google",
    emailSignIn: "Continue with email",
    orDivider: "or",
    successToast: "Signed in",
    redirectHint:
      "Finish authorization in your browser and I'll bring you right back.",

    waiting: {
      title: "Opening the browser —\nI'll wait for you to come back.",
      sub: "If the browser didn't open,",
      manualLink: "here's the manual link",
      cancel: "Cancel — I'll wait next time too",
      button: "Waiting for the browser…",
    },

    email: {
      stepEmail: {
        title: "Continue with email",
        sub: "We'll send a 6-digit code to your inbox.",
        label: "Email",
        placeholder: "you@example.com",
        submit: "Send code",
        sending: "Sending…",
        back: "Back",
        invalid: "Please enter a valid email address",
      },
      stepCode: {
        title: "Enter the code",
        sub: (email: string) => `We sent a 6-digit code to ${email}.`,
        label: "Code",
        placeholder: "6 digits",
        submit: "Sign in",
        verifying: "Verifying…",
        resend: "Resend code",
        resendIn: (seconds: number) => `Resend in ${seconds}s`,
        resending: "Resending…",
        changeEmail: "Use a different email",
        errors: {
          invalidCode: "That code doesn't match — try again.",
          codeExpired: "Code expired or not found. Request a new one.",
          tooManyAttempts: "Too many tries. Request a new code.",
          rateLimitedSend: (seconds: number) =>
            `Sending too fast — try again in ${seconds}s.`,
          sendFailed: "Couldn't send the email. Try again in a bit.",
        },
      },
    },

    failure: {
      networkTitle: "Can't reach the server from here.",
      networkDesc:
        "Check your network or try again later — I'll hold your place.",
      timeoutTitle: "Waited 60 seconds — looks like the browser drifted away.",
      timeoutDesc: "Start over.",
      cancelledTitle: "OK, come back anytime.",
      cancelledDesc: "Tap \"Continue with Google\" again when you're ready.",
      genericTitle: "Sign-in failed",
      retry: "Try again",
      details: "What went wrong?",
      deniedTitle: "Can't sign you in",
      deniedAccountLabel: "Account used:",
      deniedHint:
        "Please try again, or sign in with a different Google account.",
    },
  },

  settings: {
    dialog: {
      title: "Settings",
      description: "Preferences and connection",
    },
    sections: {
      general: "General",
      capture: "Privacy controls",
      quickAsk: "Quick Ask",
      permissions: "Permissions",
      execAgent: "Execution engine",
      skills: "Skills library",
      memory: "Memory",
      integrations: "Integrations",
      shortcuts: "Shortcuts",
      about: "About",
      developer: "Developer",
    },
    developerUnlock: {
      hint: (remaining: number) =>
        `Tap ${remaining} more time${remaining === 1 ? "" : "s"} to enable developer mode`,
      unlocked: "Developer mode enabled",
      unlockedHint: "A “Developer” tab appeared at the bottom of the sidebar.",
      alreadyOn: "Developer mode is already on",
    },
    developer: {
      title: "Developer",
      description: "Troubleshooting tools. Not needed for normal use.",
      modeGroup: "Mode",
      modeToggle: {
        label: "Keep developer mode on",
        description: "Turning this off hides the Developer tab immediately.",
      },
      devtoolsGroup: "WebView DevTools",
      devtoolsDescription:
        "Open the Chromium inspector for each window. Available in release builds.",
      devtoolsMain: "Main",
      devtoolsQuickAsk: "Quick Ask",
      devtoolsOverlay: "Notification overlay",
      devtoolsOpened: (label: string) => `Opened DevTools for ${label}`,
      devtoolsFailed: (error: string) => `Could not open DevTools: ${error}`,
      dataGroup: "Local data",
      revealDataDir: {
        label: "Reveal data directory in Finder",
        description: "Contains corivo.sqlite, config.json, captures/, logs/.",
        action: "Open",
        failed: (error: string) => `Open failed: ${error}`,
      },
      copyConfig: {
        label: "Copy current Config to clipboard",
        description:
          "byok_key and session_token are redacted; safe to paste in a bug report.",
        action: "Copy",
        success: "Copied (sensitive fields redacted)",
        failed: (error: string) => `Copy failed: ${error}`,
      },
      runtimeGroup: "Runtime",
      resetOnboarding: {
        label: "Restart onboarding",
        description: "Returns to the welcome flow. Your local data is preserved.",
        action: "Restart",
        success: "Onboarding state reset",
        failed: (error: string) => `Reset failed: ${error}`,
      },
      reloadWebview: {
        label: "Reload current window",
        description: "Equivalent to a browser refresh; the Rust process keeps running.",
        action: "Reload",
      },
    },
    general: {
      title: "General",
      description: "App behavior, startup, and UI preferences",
      startupGroup: "Startup",
      autostart: {
        label: "Launch at login",
        description: "Run Corivo automatically when your Mac starts.",
      },
      autoCapture: {
        label: "Enable context on launch",
        description: "Start context awareness immediately after Corivo opens.",
      },
      autostartUpdated: "Launch-at-login setting updated",
      windowGroup: "Window",
      minimizeToTray: {
        label: "Minimize to tray when closed",
        description:
          "Hide to the background instead of quitting when you click close.",
      },
      languageGroup: "Language",
      uiLanguage: {
        label: "UI language",
        description: "The language Corivo's interface is shown in.",
      },
      responseLanguage: {
        label: "Model response language",
        description: "The language the AI replies in inside /ask and Quick Ask.",
      },
      languageOption: {
        zh: "简体中文",
        en: "English",
      },
      appearanceGroup: "Appearance",
      theme: {
        label: "Theme",
        description: "Pick Light, Dark, or follow the system appearance.",
        options: {
          light: "Light",
          dark: "Dark",
          system: "Follow system",
        },
      },
      onboardingGroup: "Onboarding",
      resetOnboarding: {
        label: "Restart onboarding",
        description:
          "Re-runs the welcome flow. Your existing data is preserved.",
        action: "Restart",
        success: "Onboarding state reset",
      },
      dataGroup: "Data",
      retention: {
        label: "Local data retained for 90 days",
        description:
          "Context older than 90 days is pruned automatically (checked hourly). Pinned Quick Ask tasks are unaffected.",
      },
      hardDeleteGroup: "Hard delete",
      hardDelete: {
        label: "Wipe all data",
        description:
          "All context, index, and task history. Not recoverable.",
        openButton: "Destroy everything…",
        confirmTitle: "Destroy everything?",
        confirmHint: "Type DELETE to confirm.",
        confirmPlaceholder: "DELETE",
        clear: "Destroy",
        clearing: "Destroying…",
        success: (frames: number, sessions: number, screenshots: number) =>
          `Destroyed: ${frames} entries · ${sessions} sessions · ${screenshots} screenshots`,
      },
    },
    capture: {
      title: "Privacy controls",
      description:
        "Context awareness remembers your work across apps. This is your control over how that happens.",
      captureGroup: "Context awareness",
      captureCard: {
        runningTitle: "Context enabled",
        runningDesc:
          "Corivo is remembering your work. Pause anytime — it stops instantly.",
        stoppedTitle: "Paused",
        stoppedDesc:
          "Context awareness is paused. What it already remembers is still searchable in Ask.",
      },
      recent: {
        title: "Context from the last 24 hours",
        subtitle: (frames: number, size: string) =>
          `${frames.toLocaleString()} entries · ${size}`,
        window: "Past 24h",
        framesLabel: "ENTRIES",
        sizeLabel: "SIZE",
        peakLabel: "PEAK HOUR",
        histLegend: "Per-hour distribution / peak in amber",
      },
      exclusion: {
        title: "Excluded apps and websites",
        explanation:
          "Corivo won't read content from the apps and websites below — they never become part of your context. Quick Ask skips them too.",
        appsTab: "Apps",
        websitesTab: "Websites",
        defaultLabel: "Defaults (cannot be removed)",
        userLabel: "Added by you",
        userEmpty: "Nothing added yet.",
        addPlaceholder: "e.g. com.example.banking",
        adding: "Adding…",
        bundleHint:
          "To find a bundle id: select the app in Finder / Activity Monitor → File → Get Info; or run `osascript -e 'id of app \"Foo\"'` in Terminal.",
        websitesTitle: "Excluded websites",
        websitesExplanation:
          "Patterns here will be skipped when Corivo reads browser context. Use `example.com` to match the host and its subdomains; use `*.example.com` for subdomains only.",
        websitesAddPlaceholder: "e.g. notion.so or *.example.com",
        websitesEmpty: "Nothing added yet.",
        websitesComingSoonBadge: "Coming soon",
        websitesComingSoonBody:
          "Corivo is still wiring up reading the active tab's URL from your browser. Once that lands, patterns here will take effect automatically — until then, adding new ones is disabled.",
      },
    },
    quickAsk: {
      title: "Quick Ask",
      description:
        "A floating panel that hovers above any app you're in. Drop the question right where you saw it — no copy-paste, no context switch.",
      pitchTitle: "Summon and dismiss in a beat",
      pitchBody:
        "Quick Ask travels with whatever page you're looking at. Tap to bring it up, tap again to dismiss — never in your way.",
      hotkeyGroup: "Hotkey",
      hotkey: {
        title: "Trigger",
        currentLabel: "Current binding",
        bindings: {
          doubleTapOption: "Double-tap ⌥ Option",
          doubleTapAlt: "Double-tap Alt",
        },
        explanation:
          "On macOS, tap ⌥ Option twice; on Windows, tap Alt twice within 400 ms to summon Quick Ask. Tap again to dismiss. Works inside any app — including Corivo itself.",
        notInstalled:
          "Couldn't register the global listener. Restarting Corivo usually fixes it; if it persists, please file a report.",
        v1Note:
          "v1 doesn't allow custom hotkeys: system global-hotkey APIs can't bind a single modifier, so Corivo uses platform listeners for double-tap Option / Alt. Optional Cmd/Ctrl-letter shortcuts will land later.",
      },
    },
    shortcuts: {
      title: "Shortcuts",
      description:
        "Every keyboard shortcut Corivo binds — at a glance. Custom bindings aren't supported in v1.",
      statusInstalled: "Global listener active",
      statusNotInstalled: "Global listener not active",
      statusUnknown: "Status unknown",
      scopes: {
        global: "Global",
        mainWindow: "Main window",
        quickAsk: "Quick Ask panel",
        ask: "/ask conversation",
      },
      items: {
        quickAsk: {
          label: "Summon Quick Ask",
          description:
            "Double-tap the platform hotkey within 400 ms inside any app to summon Quick Ask: Option on macOS, Alt on Windows. Tap again to dismiss.",
        },
        summonMain: {
          label: "Summon main window",
          description:
            "Press from anywhere to bring the Corivo main window forward and focus it — works even when the window is minimized.",
        },
        focusSearch: {
          label: "Focus thread search",
          description:
            "Jump the cursor to the sidebar search input and select any existing query.",
        },
        clearSearch: {
          label: "Clear search",
          description:
            "Press inside the search field to clear the query and drop focus.",
        },
        closePanel: {
          label: "Close panel",
          description:
            "Dismiss the Quick Ask overlay. If a picker is open, the picker closes first.",
        },
        sendMessage: {
          label: "Send message",
          description: "Submit whatever is in the composer.",
        },
        newline: {
          label: "Insert newline",
          description: "Add a line break inside multi-line input.",
        },
        sendAsk: {
          label: "Send message",
          description: "Submit the question in the /ask composer.",
        },
      },
      keys: {
        doubleTapOption: "Double-tap ⌥",
        doubleTapAlt: "Double-tap Alt",
        cmdShiftO: "⌘ ⇧ O",
        cmdK: "⌘ K",
        esc: "Esc",
        cmdW: "⌘ W",
        escOrCmdW: "Esc or ⌘ W",
        enter: "Enter",
        shiftEnter: "⇧ Enter",
        enterOrCmdEnter: "Enter or ⌘ Enter",
      },
      footnote:
        "Bindings are fixed for now. A future version will let you customize Cmd/Ctrl-letter shortcuts.",
    },
    permissions: {
      title: "System permissions",
      description:
        "Context awareness reads the foreground window through Accessibility. Screen Recording is only used as a fallback when AX comes back empty and Corivo needs to OCR a snapshot of the window.",
      checking: "Checking…",
      granted: "Granted",
      requiredMissing: "Not granted (required)",
      optionalMissing: "Not granted (optional)",
      openSettings: "Open System Settings",
      screenRecording: {
        title: "Screen recording",
        description:
          "Optional. Used only when AX can't read the foreground app — Corivo takes a single snapshot, OCRs it, then writes the text into the frame.",
      },
      ax: {
        title: "Accessibility (AX)",
        description:
          "Recommended. Lets Corivo read foreground-window text directly through the AX API — no screenshot required.",
      },
    },
    execAgent: {
      title: "Execution engine",
      description:
        "Both /ask and Quick Ask go through the corivo-agent sidecar. Pick Corivo sign-in (recommended) or bring your own API key (BYOK).",
      authMethodGroup: "Authentication",
      modes: {
        corivo: {
          label: "Sign in with Corivo (recommended)",
          description:
            "After sign-in, Corivo provisions the model and credentials automatically — no API key configuration required.",
        },
        byok: {
          label: "Bring your own API key (advanced)",
          description:
            "Use your own Anthropic / OpenAI key, or a local OpenAI-compatible endpoint such as Ollama or vLLM.",
        },
      },
      modelGroup: "Model selection",
      mainModelLabel: "Primary model",
      refresh: "Refresh",
      refreshTitle: "Pull the latest available models from the Corivo backend",
      modelsLoading: "Loading model list…",
      modelsEmpty:
        "No models available yet. Sign in to Corivo, then click Refresh.",
      modelsRefreshed: "Model list refreshed",
      refreshFailed: (detail: string) => `Refresh failed: ${detail}`,
      thinkingBudgetGroup: "Thinking budget",
      thinkingLevelLabel: "Thinking level",
      thinkingLevels: {
        off: "Off (off)",
        minimal: "Minimal (minimal)",
        low: "Low (low)",
        medium: "Medium (medium)",
        high: "High (high)",
        xhigh: "Extra high (xhigh)",
      },
      thinkingHint:
        "Models that support thinking (Sonnet 4.6, etc.) budget reasoning tokens at this level on the main turn; unsupported models ignore it.",
      corivoLoginGroup: "Corivo sign-in",
      loggedIn: "Signed in",
      loggedOut: "Signed out",
      accountLabel: (label: string) => `Account: ${label}`,
      pleaseLogin: "Open the sign-in page to enter your invite",
      logoutCta: "Sign out",
      goLoginCta: "Go sign in",
      logoutSuccess: "Signed out of Corivo",
      byokGroup: "Bring your own API key (BYOK)",
      byokApiShapeLabel: "API shape",
      byokBaseUrlLabel: "Base URL (optional)",
      byokBaseUrlHint:
        "Leave blank to use the default. Point this at your local Ollama / vLLM / LM Studio instance for OpenAI-compatible endpoints.",
      byokKeyLabel: "API key",
      byokModelLabel: "Model id",
      byokModelHint:
        "On launch the sidecar calls the provider's /v1/models to validate the id. Mistakes surface as a specific error in Settings.",
      byokCollapse: "Collapse BYOK config",
    },
    skills: {
      title: "Skills library",
      description:
        "Symlinks Claude / Agents skills you've installed locally into Corivo's bundled Claude. Sources are auto-scanned from ~/.agents/skills/ and ~/.claude/skills/, deduplicated by name.",
      onlyCorivoMode:
        "Only the \"Use Corivo\" mode applies. The current mode does not load local skills.",
      availableTitle: "Available skills",
      checkedSummary: (enabled: number, total: number) =>
        `Selected ${enabled} / ${total}`,
      scanning: "Scanning local skill directories…",
      empty:
        "No skills found in ~/.agents/skills/ or ~/.claude/skills/. Install some, then click Rescan.",
      footnote:
        "Toggling a skill writes the symlink under $APPDATA/claude-config/skills/ instantly; the next conversation picks it up. Skills you've enabled but that have temporarily disappeared from disk stay enabled — they restore automatically when the source returns.",
    },
    about: {
      title: "About",
      description: "Version, system info, and feedback channels",
      sysInfoGroup: "System info",
      versionLabel: (version: string) => `App version: ${version}`,
      newVersionLabel: (version: string) => `New version available: ${version}`,
      osLabel: (os: string) => `OS: ${os}`,
      osVersionLabel: (version: string) => `OS version: ${version}`,
      archLabel: (arch: string) => `Arch: ${arch}`,
      tauriLabel: (version: string) => `Tauri: ${version}`,
      updating: "Updating",
      checking: "Checking",
      checkUpdate: "Check for updates",
      doUpdate: "Update",
      linksGroup: "Links",
      repo: "Source repo",
      releaseNotes: "Release notes",
      feedback: "Feedback inbox",
    },
  },

  updater: {
    failedFallback: "Update failed. Please try again later.",
    awaitingRestart:
      "The new version has been downloaded. Please quit and reopen Corivo to apply the update.",
    awaitingRestartLabel: "Awaiting restart",
  },

  workflows: {
    eyebrow: "ROUTINES",
    title: "Routines",
    description:
      "Routines are repeatable procedures Corivo has crystalized from your complex tasks — or that you've authored yourself.",
    empty: {
      title: "No routines yet",
      body:
        "Create a routine, attach a schedule, and Corivo will run it for you. You can also leave it disabled and only fire it on demand.",
    },
    capabilityFooter: {
      summary: (count: number) =>
        `${count} external capabilities connected (Lark, Claude, …)`,
      manageAction: "Open Settings",
    },
    list: {
      newAction: "+ New routine",
      enabledOn: "On",
      enabledOff: "Off",
      unscheduled: "Not scheduled",
      unscheduledHint: "Set a schedule first to enable",
      scheduleAction: "Set schedule",
      runNow: "Run now",
      cancel: "Cancel",
      running: "Running…",
      edit: "Edit",
      history: "History",
      delete: "Delete",
      neverRun: "Never run",
      lastSuccess: (when: string) => `Last success · ${when}`,
      lastFailure: (when: string) => `Last failure · ${when}`,
      nextRun: (when: string) => `Next ${when}`,
      runNowToast: "Queued",
      deleteConfirmTitle: "Delete routine?",
      deleteConfirm: (name: string) =>
        `Delete routine “${name}”? Run history will be removed too.`,
      agentBadge: "Created by Corivo",
      agentBadgeTitle:
        "Auto-created by Corivo through the schedule_task tool during a chat turn",
    },
    drawer: {
      titleCreate: "New routine",
      titleEdit: "Edit routine",
      slug: "Slug",
      slugHint: "Lowercase letters, digits, hyphens; immutable after save",
      name: "Name",
      description: "Description",
      systemPrompt: "System prompt",
      systemPromptHint: "Supports {{date}} / {{date_yesterday}} placeholders",
      tools: "Allowed tools",
      toolsHint: "Comma- or newline-separated tool names; empty = no tools",
      maxTurns: "Max turns",
      trigger: {
        title: "Schedule",
        kind: "Kind",
        interval: "Every N minutes",
        daily: "Daily",
        weekly: "Weekly",
        once: "Once",
        cron: "Cron expression",
        minutes: "Minutes",
        hour: "Hour",
        minute: "Minute",
        tz: "Timezone",
        weekdays: "Weekdays",
        at: "At",
        cronExpr: "Cron",
        cronHint: "Standard 5-field cron: min hour dom mon dow",
        preview: (when: string) => `Next: ${when}`,
        previewNone: "No upcoming firing",
        previewInvalid: "Schedule format is invalid",
      },
      enabled: "Enable immediately",
      saveAction: "Save",
      cancelAction: "Cancel",
    },
    history: {
      empty: "No runs yet",
    },
    toast: {
      openAction: "Open",
      running: "Running…",
      dismiss: "Dismiss",
    },
    sidebarSection: {
      title: "Corivo suggestions",
      empty: "No new suggestions",
      unread: (count: number) => `${count} unread`,
    },
    drawer_notify: {
      label: "Notification policy",
      always: "Notify on every run",
      onChange: "Notify only when content changes",
      silent: "Sidebar only, never push",
    },
    picker: {
      intro: "Pick a common template, or write your own.",
      backToPicker: "← Pick another template",
    },
    simple: {
      whenLabel: "When",
      reminderTextLabel: "Reminder text",
      reminderTextHint: "Corivo will send this to you at the chosen time.",
      intentLabel: "What to do",
      intentHint:
        "Describe in plain language what you want Corivo to do. This becomes the run's instructions verbatim.",
      enabledLabel: "Enable now",
      advancedToggle: "Show full fields",
      advancedHide: "Hide full fields",
    },
  },
};
