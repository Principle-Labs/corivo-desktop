// Chinese (Simplified) UI dictionary. Source of truth for the locale
// SHAPE — `LocaleDict` is derived from this object via `LooseLocale`,
// which widens string literals to `string` so the English (and any
// future) translation can substitute the actual copy without TS
// complaining about literal-type mismatches. Function-valued entries
// keep their signatures so parameter counts and types stay enforced.
//
// Strings live as plain values; parameterised messages live as functions
// (e.g. `(name) => \`欢迎 ${name}\``). Keep the tree shallow and grouped
// by feature area so a feature can be migrated end-to-end in one read.
//
// Copy style (中文):
// - 品牌词 Corivo 一律首字母大写，任何位置都用 Corivo。
// - 概念词 task 在正文中作为普通名词使用，Tasks 仅作为侧栏分组标题。
// - 中文使用全角标点 。 ， ： ； 「」 ——，不与 ASCII , : ; 混用。
// - 中英混排时，英文与中文之间留一个 ASCII 空格。
// - 数字与单位之间留空格（如 90 天 / 200 ms）。

type LooseLocale<T> = T extends string
  ? string
  : T extends (...args: infer A) => string
    ? (...args: A) => string
    : { [K in keyof T]: LooseLocale<T[K]> };

export const zh = {
  common: {
    loading: "载入中…",
    saving: "保存中…",
    cancel: "取消",
    confirm: "确认",
    delete: "删除",
    remove: "移除",
    add: "添加",
    save: "保存",
    test: "测试",
    enable: "启用",
    disable: "禁用",
    refresh: "重新扫描",
    selectAll: "全选",
    clearAll: "清空",
    unknown: "未知",
    empty: "空",
    saveFailed: (detail: string) => `保存失败：${detail}`,
    loginFailed: (detail: string) => `登录失败：${detail}`,
    deleteFailed: (detail: string) => `删除失败：${detail}`,
    addFailed: (detail: string) => `添加失败：${detail}`,
    clearFailed: (detail: string) => `清除失败：${detail}`,
    connectFailed: (detail: string) => `连接失败：${detail}`,
    deleteOpFailed: (detail: string) => `清空失败：${detail}`,
    setupFailed: (detail: string) => `设置失败：${detail}`,
    resetFailed: (detail: string) => `重置失败：${detail}`,
    logoutFailed: (detail: string) => `退出失败：${detail}`,
  },

  app: {
    booting: "Corivo 启动中…",
    forceUpdate: {
      title: "正在更新 Corivo",
      errorTitle: "更新失败",
      errorFallback: "安装新版本时出现问题。",
      preparing: "正在准备新版本安装。",
      foundVersion: (version: string) =>
        `发现新版本 ${version}，正在安装后重启。`,
      downloadProgress: (downloaded: string, total: string) =>
        `已下载 ${downloaded} / ${total}`,
      retry: "重试更新",
    },
  },

  nav: {
    ask: "提问",
  },

  sidebar: {
    workflows: "我的工作流",
    quickAskHintPrefix: "双击",
    quickAskHintSuffix: "召唤 Corivo",
    quickAskHintTitle: "在任何应用里双击 ⌥ Option，召唤 Corivo",
    balanceTitle: "查看额度并充值",
    balanceAction: "充值",
  },

  status: {
    capturing: "上下文已启用",
    stopped: "已暂停",
    disabled: "上下文已关闭",
    pausedFor: (remaining: string) => `已暂停 · 还剩 ${remaining}`,
    pending: "处理中…",
    start: "继续",
    stop: "暂停",
    intervalFormat: (seconds: number) => `${seconds}s 间隔`,
  },

  contextPopover: {
    triggerAriaLabel: "打开数据与隐私菜单",
    header: "数据与隐私",
    intro: {
      title: "上下文感知",
      body: "Corivo 会记住你在不同应用里的工作进展，不需要任何集成。",
      learnMore: "了解更多",
      dismissAria: "知道了，不再提示",
    },
    pause: {
      label: "暂停上下文感知",
      options: {
        fiveMin: "5 分钟",
        fifteenMin: "15 分钟",
        thirtyMin: "30 分钟",
        oneHour: "1 小时",
        oneDay: "1 天",
      },
      toast: (label: string) => `已暂停 ${label}`,
      toastFailed: (detail: string) => `暂停失败：${detail}`,
    },
    resume: {
      label: "立即恢复",
      toast: "已恢复",
      toastFailed: (detail: string) => `恢复失败：${detail}`,
    },
    delete: {
      label: "删除数据",
      options: {
        lastFiveMin: "最近 5 分钟",
        lastFifteenMin: "最近 15 分钟",
        custom: "自定义…",
      },
      toast: (frames: number) => `已删除 ${frames} 条上下文`,
      toastFailed: (detail: string) => `删除失败：${detail}`,
      confirm: {
        title: "删除数据",
        description:
          "选择一段时间，Corivo 会永久删除这段时间内收集到的上下文。",
        warning: "提示：此操作无法撤销，请确认后再继续。",
        durationLabel: "选择要删除的时间长度",
        durationPlaceholder: "输入时长",
        unitMinutes: "分钟",
        unitHours: "小时",
        unitDays: "天",
        confirmTypeLabel: "输入 'delete' 二次确认",
        confirmPlaceholder: "输入 delete",
        cancel: "取消",
        confirm: "删除数据",
        confirming: "正在删除…",
      },
    },
    exclusions: {
      label: "排除应用和网站",
      manage: "管理",
    },
    statusRow: {
      running: "上下文已启用",
      paused: "已暂停",
      stopped: "上下文已关闭",
    },
  },

  user: {
    defaultName: "Corivo 用户",
    comingSoon: "敬请期待",
    subscription: "订阅信息",
    settings: "设置",
    logout: "退出登录",
    logoutSuccess: "已退出登录",
    logoutConfirm: {
      title: "确认退出登录？",
      description:
        "退出后，本机的 task 与上下文不会被清除，但需要重新登录才能继续用 Corivo。",
      confirm: "退出登录",
      pending: "退出中…",
    },
  },

  billing: {
    title: "充值额度",
    description: "额度按 token 实际用量从余额扣除，余额永不过期。",
    balance: {
      label: "当前余额",
      refresh: "刷新余额",
      lastPaymentPrefix: (amount: string) => `上次充值 $${amount}`,
      status: {
        credited: "已到账",
        paid: "支付成功",
        pending: "处理中",
        failed: "支付失败",
      },
    },
    picker: {
      label: "选择充值金额",
      hint: "选择一档，数字仅为粗略估算",
      recommendedBadge: "推荐",
      tiers: {
        starter: { name: "入门", duration: "够轻度使用约一周" },
        standard: { name: "标准", duration: "够日常使用约一个月" },
        longTerm: { name: "长期", duration: "够长期使用约一个季度" },
      },
      cta: {
        idle: "选择充值金额",
        ready: (amount: number) => `去 Stripe 安全支付 $${amount}`,
        submitting: "正在跳转 Stripe…",
      },
    },
    usage: {
      label: "额度用于",
      items: {
        ask: {
          label: "/ask 长对话推理",
          hint: "包含工具调用与屏幕召回",
        },
        quickAsk: {
          label: "Quick Ask 即时问答",
          hint: "全局快捷键唤起的轻量问答",
        },
        index: {
          label: "屏幕历史的索引与摘要",
          hint: "后台把你的工作进展整理成可搜索的记忆",
        },
      },
    },
    trust: {
      primary: "通过 Stripe 安全处理 · USD 结算 · 不存储卡号",
      secondary:
        "支持 Visa · Mastercard · American Express。收据将发至你登录使用的邮箱。",
    },
    toasts: {
      openedBrowser: "已在浏览器打开支付页面",
      openedBrowserHint: "付款完成后回到 Corivo 点击刷新按钮。",
      loadFailed: "无法读取余额",
      checkoutFailed: "创建支付失败",
      refreshFailed: "刷新失败",
    },
  },

  frame: {
    drawerEyebrow: "屏幕上下文",
    loading: "载入中…",
    notFound: "找不到这条上下文。",
    unknownApp: "未知应用",
    emptyText: "这一条上下文没有提取到可读文本。",
    section: {
      text: "文字",
      data: "数据",
    },
  },

  ask: {
    threadList: {
      title: "Tasks",
      newThread: "Start with Corivo",
      empty: "还没有 task。点上面「Start with Corivo」开一个。",
      recent: "最近",
      pinned: "置顶",
      archived: "归档",
      archivedCount: (count: number) => `归档(${count})`,
      searchPlaceholder: "搜索 task",
      searchEmpty: (query: string) => `没有匹配「${query}」的 task`,
      action: {
        menu: "更多操作",
        pin: "置顶",
        unpin: "取消置顶",
        archive: "归档",
        unarchive: "恢复",
        delete: "删除",
      },
      activity: {
        running: "执行中",
        unread: "有新回复",
        error: "执行失败",
      },
      workflow: {
        // Prefix used in the sidebar in front of the workflow's
        // display name so a workflow run row reads as "⏰ 每日回顾"
        // not "每日回顾" alone. The clock glyph is inline so it
        // still works in the macOS title-bar overflow surface where
        // an icon font might not be wired up.
        prefixIcon: "⏰",
        statusSuccess: "成功",
        statusFailure: "失败",
        // Banner shown above the transcript when the viewer is in
        // read-only mode for a workflow run. Composed inline so the
        // viewer can interpolate the workflow name + when it ran.
        bannerTitle: (name: string) => `工作流：${name}`,
        bannerSubtitle: (when: string) => `${when} 运行 · 只读视图`,
        bannerManageAction: "去管理工作流",
        readOnlyHint: "工作流运行结果只读 · 如需调整请去管理页编辑或重新运行",
      },
    },
    placeholderTitle: "新 task",
    composer: {
      placeholder: "继续推进这个 task…",
      streamingPlaceholder: "响应中，请稍候…",
      sending: "推进中",
      send: "交给 Corivo",
      stop: "暂停",
      stopHint: "暂停推进",
    },
    empty: {
      title: "还没开始过 task。",
      typeHint: "在下方说一句你想做的事，或者——",
      hotkeyHint: "在任何应用里双击，Corivo 一秒就在",
    },
    welcome: "在下方说一句你想做的事。我会去 frames 里找上下文，并附上引用。",
    noActiveThread: "没有选中的 task",
    streamErrorFallback: "stream finished with error",
    permission: {
      title: "执行 Agent 请求权限",
      action: "操作：",
      reason: "原因：",
      viewDetails: "查看参数",
      hint: "允许后，Corivo 会执行此操作。如果是危险或不可逆操作，请展开参数仔细检查。",
      allow: "允许",
      deny: "拒绝",
    },
  },

  chat: {
    thinking: "思考中",
    thoughtProcess: "思考过程",
    emptyReply: "(空回复)",
    inProgress: "进行中…",
    frameAnchorTitle: "这条消息发送时关联的窗口",
    frameAnchorOne: "已关联窗口",
    frameAnchorN: (count: number) => `已关联 ${count} 个窗口`,
    youLabel: "你",
    corivoLabel: "Corivo",
    streaming: "流式中",
    completed: "已完成",
    citationsLabel: "上下文",
    citationAria: (index: number) => `第 ${index} 条上下文`,
    copy: "复制",
    copied: "已复制",
    toolVerbs: {
      recallFrames: "搜索 frames",
      runCommand: "执行命令",
      readFile: "读取文件",
      editFile: "编辑文件",
      writeFile: "写入文件",
      grep: "检索代码",
      glob: "查找文件",
      webFetch: "抓取网页",
      webSearch: "联网搜索",
      todo: "更新计划",
      timesSuffix: (n: number) => `(${n} 次)`,
      genericCall: (name: string) => `调用 ${name}`,
    },
  },

  quickAsk: {
    title: "Quick Ask",
    titleWithApp: (app: string) => `Quick Ask · ${app}`,
    closeAria: "关闭",
    closeTitle: "关闭 (Esc)",
    historyAria: "task 历史",
    historyTitle: "切换到历史 task",
    newAria: "新 task",
    newTitle: "新 task",
    inputPlaceholder: "问点什么…",
    openInAppAria: "在 App 中查看",
    openInAppTitle: "在 App 中查看这个 task",
    micAria: "语音输入(暂不可用)",
    micTitle: "语音输入(暂不可用)",
    sendAria: "发送",
    answeringAria: "回答中",
    sendTitle: "发送 (Enter)",
    answeringTitle: "回答中…",
    stopAria: "停止生成",
    stopTitle: "停止生成",
    picker: {
      newSession: "新 task",
      loading: "加载中…",
      empty: "还没有 task 历史",
      untitled: "未命名 task",
    },
    relativeTime: {
      justNow: "刚刚",
      minutesAgo: (n: number) => `${n} 分钟前`,
      hoursAgo: (n: number) => `${n} 小时前`,
      daysAgo: (n: number) => `${n} 天前`,
    },
    focus: {
      currentApp: "当前应用",
      currentWindow: "当前窗口",
      readingWindow: "正在读取窗口…",
      excludedSuffix: "已排除",
      cantReadWindow: "无法读取窗口内容",
      switchTo: (app: string) => `+ 切换到 ${app}`,
      switchTitle: "把下一条消息锚定到当前最前的应用",
    },
    titlebarHint: "Esc",
    resizeAria: "调整窗口大小",
    resizeTitle: "拖动调整窗口大小",
  },

  onboarding: {
    steps: {
      permission: "授权",
      demo: "试一试",
      shortcut: "快捷唤醒",
    },
    nav: {
      back: "上一步",
      continue: "继续",
    },
    permission: {
      stepLabel: "01 · 授权",
      title: "授权 Corivo",
      subtitle: "Corivo 需要两项 macOS 权限来理解你的上下文。",
      ax: {
        name: "辅助功能",
        desc: "用于读取屏幕上的文字",
      },
      screen: {
        name: "屏幕录制",
        desc: "用于识别屏幕上的文字",
      },
      status: {
        pending: "待授权",
        asking: "请求中…",
        granted: "已授权",
      },
      grant: "授权",
      foot: "两项授权完成后将自动进入下一步\n数据将被转成结构化文本存储在你电脑本地",
      denied:
        "macOS 不会再次弹出对话框。请到「系统设置 → 隐私与安全性」找到 Corivo 打开开关，Corivo 会自动检测到。",
      openSettings: "打开系统设置",
    },
    demo: {
      stepLabel: "02 · 试一试",
      title: "试试 Corivo 的用法",
      subtitle: "下方是一份示例文档 —— Corivo 已经看到了它。你直接提问就行。",
      url: "演示 · 2026 Q3 产品规划",
      doc: {
        title: "2026 Q3 产品规划",
        meta: "Alex Chen · Product · 2026-03-15",
        bgH: "背景",
        bgP: "过去一个季度，活跃用户增长 32%，但新用户首周留存仅 41%，远低于行业基准。核心问题集中在 onboarding 流程过长、产品价值传达不清。",
        goalH: "目标",
        goalP: "把新用户首周留存提升到 55%，onboarding 完成率从 68% 提升到 80%。",
        planH: "方案",
        plan1: "重做 onboarding 三步流程，去掉冗余环节",
        plan2: "第一步聚焦权限请求，附清晰隐私承诺",
        plan3: "第二步用真实场景演示产品能力",
        plan4: "第三步介绍快捷键唤起方式",
        msH: "里程碑",
        ms1: "7 月 15 日：设计完成 + 评审",
        ms2: "8 月 1 日：开发完成，进入内测",
        ms3: "8 月 20 日：全量上线",
        ms4: "9 月底：数据复盘，决定下一轮迭代",
        riskH: "风险",
        riskP: "权限请求页是高敏感环节，需要 A/B 测试两种文案策略。",
      },
      askPlaceholder: "问问刚才屏幕上看到的内容…",
      chip: "总结我正在看的这份文档，存到 Apple Notes",
      cta: "继续",
      skip: "跳过",
      askTitle: "试试 Corivo",
      send: "发送",
      stop: "停止",
      retry: "重试",
      thinking: "正在思考…",
      you: "你",
      corivoLabel: "Corivo",
      errorLabel: "出错了",
      networkErrorHint: "网络异常或服务暂时不可用，请检查连接后重试。",
    },
    shortcut: {
      stepLabel: "03 · 快捷唤醒",
      title: "连按两下 Option，快速唤醒 Corivo",
      subtitle:
        "正常使用电脑就行，任何时候都可以双击 Option 键唤起 Corivo。",
      cue: "连按 2 下 ⌥",
      cta: "进入 Corivo",
    },
  },

  login: {
    // Page-level brand framing — left column on the redesigned login.
    brand: "Corivo",
    tagline: "真正理解你工作的 Agent",
    // Right column — the actual auth.
    eyebrow: "Sign in",
    title: "登录 Corivo",
    googleSignIn: "使用 Google 登录",
    emailSignIn: "使用邮箱登录",
    orDivider: "或",
    successToast: "登录成功",
    redirectHint: "请在浏览器中完成授权",

    // Waiting state shown during the OAuth browser round-trip.
    waiting: {
      title: "请在浏览器中完成登录",
      sub: "如果浏览器未自动打开，",
      manualLink: "请点击此处",
      cancel: "取消登录",
      // Compatibility with the older Button label in case anyone
      // still maps to it.
      button: "等待授权…",
    },

    // Email OTP — the two-step inline form (邮箱 → 验证码)
    email: {
      stepEmail: {
        title: "用邮箱继续",
        sub: "我们会把 6 位验证码发到你的邮箱。",
        label: "邮箱",
        placeholder: "you@example.com",
        submit: "发送验证码",
        sending: "发送中…",
        back: "返回",
        invalid: "请输入有效的邮箱地址",
      },
      stepCode: {
        title: "输入验证码",
        sub: (email: string) => `我们已发送一封带 6 位验证码的邮件到 ${email}。`,
        label: "验证码",
        placeholder: "6 位数字",
        submit: "登录",
        verifying: "验证中…",
        resend: "重发验证码",
        resendIn: (seconds: number) => `${seconds} 秒后可重发`,
        resending: "重新发送中…",
        changeEmail: "换一个邮箱",
        // Inline error messages keyed off the server's `code` field.
        errors: {
          invalidCode: "验证码不正确，请重试。",
          codeExpired: "验证码已过期或不存在，请重新获取。",
          tooManyAttempts: "尝试次数过多，请重新获取验证码。",
          rateLimitedSend: (seconds: number) =>
            `发送过于频繁，请 ${seconds} 秒后再试。`,
          sendFailed: "邮件发送失败，请稍后重试。",
        },
      },
    },

    // Inline failure cards — replace the toast-driven error path.
    failure: {
      networkTitle: "网络连接失败",
      networkDesc: "请检查网络后重试。",
      timeoutTitle: "登录超时",
      timeoutDesc: "授权未在限定时间内完成，请重新登录。",
      cancelledTitle: "已取消登录",
      cancelledDesc: "如需登录，请重新点击「使用 Google 登录」。",
      genericTitle: "登录失败",
      retry: "重试",
      details: "查看详情",
      // Auth-denied error coming from the closed-beta whitelist
      // (kept for back-compat — closed beta is being lifted, but the
      // server may still emit `auth_denied` for shadow-banned accounts).
      deniedTitle: "无法登录",
      deniedAccountLabel: "使用账户：",
      deniedHint: "请稍后重试，或更换 Google 账户登录。",
    },
  },

  settings: {
    dialog: {
      title: "设置",
      description: "偏好与连接配置",
    },
    sections: {
      general: "通用",
      capture: "隐私控制",
      quickAsk: "Quick Ask",
      permissions: "权限",
      execAgent: "执行引擎",
      skills: "技能库",
      memory: "记忆",
      integrations: "集成",
      shortcuts: "快捷键",
      about: "关于",
      developer: "开发者",
    },
    developerUnlock: {
      hint: (remaining: number) => `再点击 ${remaining} 次进入开发者模式`,
      unlocked: "已进入开发者模式",
      unlockedHint: "侧栏底部多了「开发者」入口。",
      alreadyOn: "开发者模式已开启",
    },
    developer: {
      title: "开发者",
      description: "用于排查问题的工具，普通使用不需要。",
      modeGroup: "模式",
      modeToggle: {
        label: "保留开发者模式",
        description: "关掉后侧栏的「开发者」入口立即消失。",
      },
      devtoolsGroup: "WebView DevTools",
      devtoolsDescription: "对三个窗口分别打开 Chromium 检查器。release 包默认也可用。",
      devtoolsMain: "主窗口",
      devtoolsQuickAsk: "Quick Ask",
      devtoolsOverlay: "通知浮层",
      devtoolsOpened: (label: string) => `已打开 ${label} 的 DevTools`,
      devtoolsFailed: (error: string) => `无法打开 DevTools：${error}`,
      dataGroup: "本地数据",
      revealDataDir: {
        label: "在 Finder 中显示数据目录",
        description: "包含 corivo.sqlite、config.json、captures/、logs/。",
        action: "打开",
        failed: (error: string) => `打开失败：${error}`,
      },
      copyConfig: {
        label: "复制当前 Config 到剪贴板",
        description: "已自动隐去 byok_key 与 session_token，可直接贴出来排查。",
        action: "复制",
        success: "已复制（敏感字段已脱敏）",
        failed: (error: string) => `复制失败：${error}`,
      },
      runtimeGroup: "运行时",
      resetOnboarding: {
        label: "重新跑引导",
        description: "回到欢迎向导。不会清除你的本地数据。",
        action: "重新跑",
        success: "已重置引导状态",
        failed: (error: string) => `重置失败：${error}`,
      },
      reloadWebview: {
        label: "重新加载当前窗口",
        description: "等同于浏览器刷新；不会重启 Rust 进程。",
        action: "重新加载",
      },
    },
    general: {
      title: "通用",
      description: "应用行为、启动和界面偏好",
      startupGroup: "启动行为",
      autostart: {
        label: "开机自启动",
        description: "电脑启动时自动运行 Corivo。",
      },
      autoCapture: {
        label: "启动时自动开启上下文",
        description: "Corivo 启动后立即开启上下文感知。",
      },
      autostartUpdated: "已更新开机自启设置",
      windowGroup: "窗口",
      minimizeToTray: {
        label: "关闭时最小化到托盘",
        description: "点击关闭按钮时隐藏到后台而不是退出。",
      },
      languageGroup: "语言",
      uiLanguage: {
        label: "界面语言",
        description: "Corivo 应用界面的显示语言。",
      },
      responseLanguage: {
        label: "模型回复语言",
        description: "AI 在 /ask 与 Quick Ask 中回复你时使用的语言。",
      },
      languageOption: {
        zh: "简体中文",
        en: "English",
      },
      appearanceGroup: "外观",
      theme: {
        label: "主题",
        description: "选择浅色、深色，或跟随系统外观。",
        options: {
          light: "浅色",
          dark: "深色",
          system: "跟随系统",
        },
      },
      onboardingGroup: "引导",
      resetOnboarding: {
        label: "重新跑引导",
        description: "重新进入欢迎向导，不会清除你已有的数据。",
        action: "重新跑",
        success: "已重置引导状态",
      },
      dataGroup: "数据",
      retention: {
        label: "本地数据保留 90 天",
        description:
          "超过 90 天的上下文与索引会自动清理(每小时检查一次)。Quick Ask 保留的对话不受影响。",
      },
      hardDeleteGroup: "硬删除",
      hardDelete: {
        label: "清空所有数据",
        description: "所有上下文、索引、task 历史。不可恢复。",
        openButton: "全部销毁…",
        confirmTitle: "确认全部销毁？",
        confirmHint: "输入 DELETE 二次确认。",
        confirmPlaceholder: "DELETE",
        clear: "全部销毁",
        clearing: "正在销毁…",
        success: (frames: number, sessions: number, screenshots: number) =>
          `已销毁： ${frames} 条上下文 · ${sessions} 个 session · ${screenshots} 张截图`,
      },
    },
    capture: {
      title: "隐私控制",
      description:
        "上下文感知会记住你在不同应用里的工作进展。下面是你对这件事的全部控制权。",
      captureGroup: "上下文感知",
      captureCard: {
        runningTitle: "上下文已启用",
        runningDesc:
          "Corivo 正在记住你的工作进展。任何时候按「暂停」都立刻停。",
        stoppedTitle: "已暂停",
        stoppedDesc:
          "上下文感知已暂停。之前记住的内容仍然可以在 Ask 里查到。",
      },
      recent: {
        title: "最近 24 小时的上下文",
        subtitle: (frames: number, size: string) =>
          `${frames.toLocaleString()} 条 · ${size}`,
        window: "过去 24h",
        framesLabel: "条数",
        sizeLabel: "体积",
        peakLabel: "高峰时段",
        histLegend: "按小时分布 / 高峰用琥珀标出",
      },
      exclusion: {
        title: "排除应用和网站",
        explanation:
          "下面列出的应用和网站，Corivo 不会读它们的内容，也不会把它们纳入上下文。Quick Ask 也会跳过它们。",
        appsTab: "应用",
        websitesTab: "网站",
        defaultLabel: "默认(不可移除)",
        userLabel: "你添加的",
        userEmpty: "还没有添加。",
        addPlaceholder: "例： com.example.banking",
        adding: "添加中…",
        bundleHint:
          "要找 bundle id:在 Finder / Activity Monitor 选中应用，菜单栏 File → Get Info;或在终端 osascript -e 'id of app \"Foo\"'。",
        websitesTitle: "排除的网站",
        websitesExplanation:
          "未来你可以在这里加规则，Corivo 在读取浏览器上下文时会跳过命中的网站。直接写 example.com 同时匹配主机和子域名；写 *.example.com 只匹配子域名。",
        websitesAddPlaceholder: "例： notion.so 或 *.example.com",
        websitesEmpty: "还没有添加。",
        websitesComingSoonBadge: "即将上线",
        websitesComingSoonBody:
          "Corivo 还在打磨从浏览器读取当前网址的能力。等这一步落地，这里的规则就会自动生效 —— 暂时无法添加。",
      },
    },
    quickAsk: {
      title: "Quick Ask",
      description:
        "在任何应用之上呼出 Corivo 的小窗口，把屏幕上的问题就地交给我。不打断正在做的事，问完即走。",
      pitchTitle: "随时召唤，随时收起",
      pitchBody:
        "Quick Ask 会带着你当前看的页面一起来 —— 不必复制粘贴。一秒呼出，再按一次就消失。",
      hotkeyGroup: "快捷键",
      hotkey: {
        title: "唤起方式",
        currentLabel: "当前快捷键",
        bindings: {
          doubleTapOption: "双击 ⌥ Option",
        },
        explanation:
          "连按两次 ⌥ Option 键（间隔 400 ms 内）即可唤起 Quick Ask 面板；再按一次会收起。无论你在哪个应用里都能触发，包括 Corivo 自己。",
        notInstalled:
          "未能注册全局监听。重启 Corivo 通常可恢复；持续失败请提交反馈。",
        v1Note:
          "v1 暂不支持自定义快捷键：macOS 的 RegisterEventHotKey 不允许把单个修饰键注册为热键，所以我们改用 NSEvent flagsChanged 监听双击 ⌥。后续会引入可选的 Cmd/Ctrl + 字母 快捷键模式。",
      },
    },
    shortcuts: {
      title: "快捷键",
      description: "Corivo 在用的全部快捷键 —— 一张表能看全。v1 暂不支持自定义。",
      statusInstalled: "全局监听已就绪",
      statusNotInstalled: "全局监听未就绪",
      statusUnknown: "状态未知",
      scopes: {
        global: "全局",
        mainWindow: "主窗口",
        quickAsk: "Quick Ask 面板",
        ask: "/ask 对话",
      },
      items: {
        quickAsk: {
          label: "唤起 Quick Ask",
          description:
            "在任何应用里 400 ms 内双击 ⌥ Option 召唤 Quick Ask；再按一次收起。",
        },
        summonMain: {
          label: "唤起主窗口",
          description:
            "在任何应用里按下都会把 Corivo 主窗口拉到前台并聚焦；窗口最小化时也能直接召回。",
        },
        focusSearch: {
          label: "聚焦会话搜索",
          description: "把光标跳到侧边栏的搜索输入框，并选中已有内容。",
        },
        clearSearch: {
          label: "清空搜索",
          description: "在搜索框内按下，清空查询并取消聚焦。",
        },
        closePanel: {
          label: "关闭面板",
          description: "收起 Quick Ask 浮窗。若有候选下拉，先关下拉。",
        },
        sendMessage: {
          label: "发送消息",
          description: "提交当前输入框的内容。",
        },
        newline: {
          label: "插入换行",
          description: "在多行输入中加一个换行。",
        },
        sendAsk: {
          label: "发送消息",
          description: "在 /ask 对话框内提交问题。",
        },
      },
      keys: {
        doubleTapOption: "双击 ⌥",
        cmdShiftO: "⌘ ⇧ O",
        cmdK: "⌘ K",
        esc: "Esc",
        cmdW: "⌘ W",
        escOrCmdW: "Esc 或 ⌘ W",
        enter: "Enter",
        shiftEnter: "⇧ Enter",
        enterOrCmdEnter: "Enter 或 ⌘ Enter",
      },
      footnote:
        "目前快捷键全部固定，无法在界面里修改。后续版本会开放自定义 Cmd/Ctrl + 字母 的绑定。",
    },
    permissions: {
      title: "系统权限",
      description:
        "上下文感知通过辅助功能 (AX) 读前台窗口正文。屏幕录制权限只在 AX 拿不到正文时才用 —— Corivo 会截一张图、OCR 出文字，再写进 frame。",
      checking: "检查中…",
      granted: "已授权",
      requiredMissing: "未授权(必须)",
      optionalMissing: "未授权(可选)",
      openSettings: "打开系统设置",
      screenRecording: {
        title: "屏幕录制",
        description:
          "可选。仅在 AX 拿不到前台正文时使用 —— Corivo 截一张图、OCR 出文字、写进 frame，平时不抓屏。",
      },
      ax: {
        title: "辅助功能 (AX)",
        description:
          "推荐开启。让 Corivo 直接通过 AX 接口读前台窗口正文，不需要截图。",
      },
    },
    execAgent: {
      title: "执行引擎",
      description:
        "所有 chat(/ask 与 Quick Ask)都走 corivo-agent sidecar。可选 Corivo 登录(推荐)或自带 API Key(BYOK)。",
      authMethodGroup: "认证方式",
      modes: {
        corivo: {
          label: "使用 Corivo 登录(推荐)",
          description:
            "登录后自动使用 Corivo 服务下发的模型与凭据，无需配置 API key。",
        },
        byok: {
          label: "自带 API Key (高级)",
          description:
            "使用自己的 Anthropic / OpenAI key，或本地 Ollama / vLLM 等 OpenAI 兼容服务。",
        },
      },
      modelGroup: "模型选择",
      mainModelLabel: "主模型",
      refresh: "刷新",
      refreshTitle: "从 Corivo 后端重新拉取可用模型",
      modelsLoading: "正在加载模型列表…",
      modelsEmpty:
        "尚未拉取到模型列表。请先登录 Corivo，再点击右上角的「刷新」。",
      modelsRefreshed: "模型列表已刷新",
      refreshFailed: (detail: string) => `刷新失败：${detail}`,
      thinkingBudgetGroup: "思考预算",
      thinkingLevelLabel: "Thinking Level",
      thinkingLevels: {
        off: "关闭 (off)",
        minimal: "最低 (minimal)",
        low: "低 (low)",
        medium: "中 (medium)",
        high: "高 (high)",
        xhigh: "极高 (xhigh)",
      },
      thinkingHint:
        "支持 thinking 的模型(Sonnet 4.6 等)在主回合按此级别预算 reasoning tokens;不支持的模型会直接忽略。",
      corivoLoginGroup: "Corivo 登录",
      loggedIn: "已登录",
      loggedOut: "未登录",
      accountLabel: (label: string) => `账号：${label}`,
      pleaseLogin: "请前往登录页输入邀请码",
      logoutCta: "退出登录",
      goLoginCta: "前往登录",
      logoutSuccess: "已退出 Corivo 登录",
      byokGroup: "自带 API Key (BYOK)",
      byokApiShapeLabel: "API Shape",
      byokBaseUrlLabel: "Base URL (可选)",
      byokBaseUrlHint:
        "留空使用默认。本地 Ollama / vLLM / LM Studio 等 OpenAI 兼容服务在这里填它们的 base url。",
      byokKeyLabel: "API Key",
      byokModelLabel: "Model id",
      byokModelHint:
        "sidecar 启动时会调 provider 的 /v1/models 校验 id 合法性。填错时设置页会显示具体错误。",
      byokCollapse: "收起 BYOK 配置",
    },
    skills: {
      title: "技能库",
      description:
        "把本机已安装的 Claude / Agents skill 通过 symlink 暴露给 Corivo 内置的执行引擎。来源自动从 ~/.agents/skills/ 与 ~/.claude/skills/ 扫描，按名称去重。",
      onlyCorivoMode:
        "仅 “使用 Corivo 配置” 模式生效。当前模式不会启用本机 skill。",
      availableTitle: "可用 skill",
      checkedSummary: (enabled: number, total: number) =>
        `已勾选 ${enabled} / ${total}`,
      scanning: "正在扫描本机 skill 目录…",
      empty:
        "未在 ~/.agents/skills/ 或 ~/.claude/skills/ 找到 skill。安装后点 “重新扫描”。",
      footnote:
        "勾选后立即写入 $APPDATA/claude-config/skills/ 的 symlink;下次对话即生效。已勾选但本机暂时不存在的 skill 会保留勾选状态，等 skill 重新出现自动恢复。",
    },
    about: {
      title: "关于",
      description: "版本、系统信息与反馈渠道",
      sysInfoGroup: "系统信息",
      versionLabel: (version: string) => `应用版本：${version}`,
      newVersionLabel: (version: string) => `发现新版本：${version}`,
      osLabel: (os: string) => `系统：${os}`,
      osVersionLabel: (version: string) => `系统版本：${version}`,
      archLabel: (arch: string) => `架构：${arch}`,
      tauriLabel: (version: string) => `Tauri:${version}`,
      updating: "更新中",
      checking: "检查中",
      checkUpdate: "检查更新",
      doUpdate: "更新",
      linksGroup: "链接",
      repo: "开源仓库",
      releaseNotes: "发布记录",
      feedback: "反馈邮箱",
    },
  },

  updater: {
    failedFallback: "更新失败，请稍后重试。",
    awaitingRestart: "新版本已下载完成，请退出软件后重新打开以应用更新。",
    awaitingRestartLabel: "等待重启",
  },

  workflows: {
    eyebrow: "MY WORKFLOWS",
    title: "我的工作流",
    description:
      "工作流是 Corivo 帮你固化下来的可重复使用过程 —— 来自你做过的复杂任务、或你亲自定义的步骤。",
    empty: {
      title: "还没有工作流",
      body:
        "新建一条工作流，给它定个执行时间，Corivo 就会按时替你跑。也可以先建好不启用，临时点「立即运行」按需触发。",
    },
    capabilityFooter: {
      summary: (count: number) =>
        `已连接 ${count} 项外部能力（飞书、Claude 等）`,
      manageAction: "去设置",
    },
    list: {
      newAction: "+ 新建工作流",
      enabledOn: "已启用",
      enabledOff: "未启用",
      unscheduled: "未排程",
      unscheduledHint: "需要先设置执行时间才能启用",
      scheduleAction: "设置时间",
      runNow: "立即运行",
      cancel: "取消",
      running: "正在运行…",
      edit: "编辑",
      history: "查看历史",
      delete: "删除",
      neverRun: "尚未运行",
      lastSuccess: (when: string) => `上次成功 · ${when}`,
      lastFailure: (when: string) => `上次失败 · ${when}`,
      nextRun: (when: string) => `下次 ${when}`,
      runNowToast: "已加入运行队列",
      deleteConfirmTitle: "删除工作流？",
      deleteConfirm: (name: string) => `确定要删除工作流「${name}」吗？运行历史会一并删除。`,
      agentBadge: "Corivo 自动创建",
      agentBadgeTitle: "由 Corivo 在对话中通过 schedule_task 自动创建",
    },
    drawer: {
      titleCreate: "新建工作流",
      titleEdit: "编辑工作流",
      slug: "标识 (slug)",
      slugHint: "小写字母、数字、连字符；保存后不可更改",
      name: "名字",
      description: "描述",
      systemPrompt: "系统提示",
      systemPromptHint: "支持 {{date}} / {{date_yesterday}} 占位符",
      tools: "允许的工具",
      toolsHint: "逗号或换行分隔工具名，留空则不调用任何工具",
      maxTurns: "最大轮次",
      trigger: {
        title: "执行时间",
        kind: "类型",
        interval: "每隔 N 分钟",
        daily: "每天",
        weekly: "每周",
        once: "一次性",
        cron: "Cron 表达式",
        minutes: "分钟数",
        hour: "时",
        minute: "分",
        tz: "时区",
        weekdays: "周几",
        at: "时刻",
        cronExpr: "Cron",
        cronHint: "5 字段标准 cron：分 时 日 月 周",
        preview: (when: string) => `下次将在 ${when}`,
        previewNone: "暂无未来触发时间",
        previewInvalid: "时间格式无效",
      },
      enabled: "立即启用",
      saveAction: "保存",
      cancelAction: "取消",
    },
    history: {
      // `empty` survives the v3 unified-history refactor (used by the
      // workflows page's "查看历史" toast when no run exists yet).
      // The old title/status keys lived inside `WorkflowHistoryDialog`
      // — now deleted; equivalents moved to
      // `ask.threadList.workflow.statusSuccess` / `.statusFailure`.
      empty: "还没有运行记录",
    },
    toast: {
      openAction: "查看",
      running: "正在运行…",
      dismiss: "收起",
    },
    sidebarSection: {
      title: "Corivo 提议",
      empty: "暂无新提议",
      unread: (count: number) => `${count} 条未读`,
    },
    drawer_notify: {
      label: "通知策略",
      always: "每次都通知",
      onChange: "只在内容变化时通知",
      silent: "只进侧栏，不弹通知",
    },
    picker: {
      intro: "挑一个常用模板，或者自己写一个。",
      backToPicker: "← 换一个模板",
    },
    simple: {
      whenLabel: "什么时候",
      reminderTextLabel: "提醒内容",
      reminderTextHint: "Corivo 会在到点时把这句话发给你。",
      intentLabel: "做什么",
      intentHint: "用自然语言告诉 Corivo 你要它做的事。Corivo 会按这段执行。",
      enabledLabel: "立即启用",
      advancedToggle: "显示完整字段",
      advancedHide: "收起完整字段",
    },
  },
} as const;

/** Shape of a locale dictionary: same nesting and signatures as `zh`,
 *  with string values widened from literals to `string` so other
 *  languages can supply their own copy without literal-type mismatch. */
export type LocaleDict = LooseLocale<typeof zh>;
