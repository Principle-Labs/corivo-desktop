import { useEffect, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { ArrowLeft, ArrowRight } from "lucide-react";

import { Button } from "@repo/ui/components/button";
import { Input } from "@repo/ui/components/input";
import { Label } from "@repo/ui/components/label";

import { Mascot } from "@/components/brand/mascot";
import { TitleBar } from "@/components/layout/title-bar";
import { useTranslation } from "@/i18n";
import {
  authLoginEmail,
  authLoginGoogle,
  authRequestEmailCode,
  authStatus,
  getOnboardingState,
} from "@/lib/tauri";
import type { TauriError } from "@/lib/types";
import { applyAuthStatus } from "@/stores/user-profile-store";
import type { EmailRejectCode } from "@corivo/shared-types";

type AuthDeniedError = Extract<TauriError, { kind: "auth_denied" }>;

/** Coarse classification of what went wrong during the OAuth round-trip.
 *  Each branch lights up its own inline failure card with humanity-tuned
 *  copy — beats a generic toast for the rare-but-bad path of "I clicked
 *  sign in and the world fell over". */
type FailureKind =
  | { kind: "denied"; error: AuthDeniedError }
  | { kind: "network" }
  | { kind: "timeout" }
  | { kind: "cancelled" }
  | { kind: "generic"; message: string };

function isAuthDenied(err: unknown): err is AuthDeniedError {
  return (
    typeof err === "object" &&
    err !== null &&
    "kind" in err &&
    (err as { kind: unknown }).kind === "auth_denied"
  );
}

function extractMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  if (
    typeof err === "object" &&
    err !== null &&
    "message" in err &&
    typeof (err as { message: unknown }).message === "string"
  ) {
    return (err as { message: string }).message;
  }
  return JSON.stringify(err);
}

function classifyFailure(err: unknown): FailureKind {
  if (isAuthDenied(err)) return { kind: "denied", error: err };
  const message = extractMessage(err).toLowerCase();
  if (
    message.includes("network") ||
    message.includes("offline") ||
    message.includes("dns") ||
    message.includes("fetch") ||
    message.includes("connection")
  ) {
    return { kind: "network" };
  }
  if (message.includes("timeout") || message.includes("timed out")) {
    return { kind: "timeout" };
  }
  if (message.includes("cancel") || message.includes("aborted")) {
    return { kind: "cancelled" };
  }
  return { kind: "generic", message: extractMessage(err) };
}

/** UI-level mode for the right-column auth surface. `idle` is the
 *  entry choice (Google + 邮箱); `emailStep1` collects the email and
 *  posts request-code; `emailStep2` collects the OTP code. The Google
 *  loopback flow lives entirely inside `submitting` (no dedicated
 *  mode). */
type AuthMode = "idle" | "emailStep1" | "emailStep2";

/// One-button sign-in (Google) + email-OTP fallback.
///
/// Layout: two-column per docs/design/auth-onboarding-v0.html v0.2.
///   Left column  · brand lockup + Quick Ask "形态浮影" video + tagline.
///   Right column · Sign-in CTA (Google + email) + waiting state +
///                  inline failure cards + email two-step inline form.
export function LoginPage() {
  const navigate = useNavigate();
  const [mode, setMode] = useState<AuthMode>("idle");
  const [submitting, setSubmitting] = useState(false);
  const [failure, setFailure] = useState<FailureKind | null>(null);

  // Email OTP state. Hoisted to LoginPage so going back/forward
  // between step1 ↔ step2 doesn't lose user input.
  const [emailDraft, setEmailDraft] = useState("");
  const [confirmedEmail, setConfirmedEmail] = useState("");

  // If we landed on /login with a live session already (e.g. user
  // navigated here manually after signing in elsewhere), immediately
  // route them onward. Honors onboarding gating.
  useEffect(() => {
    let cancelled = false;
    authStatus()
      .then(async (status) => {
        if (cancelled) return;
        if (status.loggedIn) {
          const onboarding = await getOnboardingState().catch(() => null);
          if (cancelled) return;
          if (onboarding?.needs_onboarding) {
            void navigate({ to: "/onboarding/permission", replace: true });
          } else {
            void navigate({ to: "/ask", replace: true });
          }
        }
      })
      .catch(() => {
        // Service not yet ready (rare race with app boot) — show the
        // form anyway. The submit handler will report the real error.
      });
    return () => {
      cancelled = true;
    };
  }, [navigate]);

  const navigateAfterLogin = async () => {
    const onboarding = await getOnboardingState();
    if (onboarding.needs_onboarding) {
      await navigate({ to: "/onboarding/permission", replace: true });
    } else {
      await navigate({ to: "/ask", replace: true });
    }
  };

  const handleGoogleSignIn = async () => {
    if (submitting) return;
    setSubmitting(true);
    setFailure(null);
    try {
      const status = await authLoginGoogle();
      applyAuthStatus(status);
      await navigateAfterLogin();
    } catch (error) {
      setFailure(classifyFailure(error));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="relative flex h-screen flex-col overflow-hidden bg-background text-foreground antialiased">
      <TitleBar />
      <main className="grid flex-1 min-h-0 overflow-hidden grid-cols-[1.18fr_1fr]">
        <LoginVisual />
        <LoginAuth
          mode={mode}
          submitting={submitting}
          failure={failure}
          emailDraft={emailDraft}
          confirmedEmail={confirmedEmail}
          onModeChange={setMode}
          onGoogleSignIn={handleGoogleSignIn}
          onEmailDraftChange={setEmailDraft}
          onEmailCodeSent={(email) => {
            setConfirmedEmail(email);
            setMode("emailStep2");
          }}
          onEmailLoggedIn={async (status) => {
            applyAuthStatus(status);
            await navigateAfterLogin();
          }}
          onClearFailure={() => setFailure(null)}
          onReportFailure={setFailure}
        />
      </main>
    </div>
  );
}

/**
 * Left column · brand lockup + product-form preview + tagline.
 *
 * The hero is a silently-looping MP4 of the Quick Ask 浮层 ("形态浮影")
 * — only the shape, never any real content. Hosted on R2 so it
 * doesn't bloat the desktop bundle.
 */
function LoginVisual() {
  const { t } = useTranslation();
  return (
    <section className="relative flex min-h-0 flex-col gap-6 border-r border-border bg-gradient-to-b from-muted to-[var(--bg-deep,var(--secondary))] px-12 py-10 dark:from-[color-mix(in_oklab,var(--card)_92%,transparent)] dark:to-[color-mix(in_oklab,var(--secondary)_94%,transparent)]">
      <div className="flex shrink-0 items-center gap-2.5">
        <Mascot size="sm" />
        <span className="font-display text-[15px] font-semibold leading-none tracking-[-0.01em] text-foreground">
          {t.login.brand}
        </span>
      </div>

      <div className="relative flex min-h-0 flex-1 flex-col items-center justify-center gap-6 overflow-hidden">
        <LoginHeroMark />
        <p
          className="w-[360px] max-w-full shrink-0 font-display text-[22px] font-semibold leading-[1.3] tracking-[-0.018em] text-foreground"
          style={{ textWrap: "balance" }}
        >
          {t.login.tagline}
        </p>
      </div>
    </section>
  );
}

function LoginHeroMark() {
  return (
    <div
      aria-hidden="true"
      className="flex aspect-[3/4] h-full max-h-[480px] w-auto items-center justify-center rounded-[10px] border border-[color-mix(in_oklab,var(--foreground)_13%,transparent)] bg-[var(--bg-deep,var(--secondary))] p-10 shadow-md"
    >
      <Mascot size="lg" />
    </div>
  );
}

/**
 * Right column · auth surface. Exclusive states (priority top-down):
 *   1. failure — dedicated inline card with retry CTA
 *   2. submitting (Google waiting only)
 *   3. emailStep1 / emailStep2 — email OTP two-step form
 *   4. idle — Google CTA + email CTA
 */
function LoginAuth(props: {
  mode: AuthMode;
  submitting: boolean;
  failure: FailureKind | null;
  emailDraft: string;
  confirmedEmail: string;
  onModeChange: (mode: AuthMode) => void;
  onGoogleSignIn: () => void;
  onEmailDraftChange: (value: string) => void;
  onEmailCodeSent: (email: string) => void;
  onEmailLoggedIn: (status: Parameters<typeof applyAuthStatus>[0]) => void;
  onClearFailure: () => void;
  onReportFailure: (failure: FailureKind) => void;
}) {
  if (props.failure) {
    return (
      <FailurePane
        failure={props.failure}
        onRetry={() => {
          props.onClearFailure();
          // Failure pane is only used by Google today — re-trigger it.
          props.onGoogleSignIn();
        }}
      />
    );
  }

  if (props.submitting) {
    return <WaitingPane />;
  }

  if (props.mode === "emailStep1") {
    return (
      <EmailStep1Pane
        emailDraft={props.emailDraft}
        onEmailDraftChange={props.onEmailDraftChange}
        onBack={() => props.onModeChange("idle")}
        onSent={props.onEmailCodeSent}
        onReportFailure={props.onReportFailure}
      />
    );
  }

  if (props.mode === "emailStep2") {
    return (
      <EmailStep2Pane
        email={props.confirmedEmail}
        onChangeEmail={() => props.onModeChange("emailStep1")}
        onLoggedIn={props.onEmailLoggedIn}
        onReportFailure={props.onReportFailure}
      />
    );
  }

  return (
    <IdlePane
      onGoogleSignIn={props.onGoogleSignIn}
      onEmailEntry={() => props.onModeChange("emailStep1")}
    />
  );
}

function IdlePane({
  onGoogleSignIn,
  onEmailEntry,
}: {
  onGoogleSignIn: () => void;
  onEmailEntry: () => void;
}) {
  const { t } = useTranslation();
  return (
    <section className="flex flex-col items-center justify-center bg-background px-14 py-10">
      <div className="flex w-full max-w-[340px] flex-col items-start gap-[18px]">
        <span className="font-mono text-[10px] uppercase tracking-[0.18em] text-muted-foreground">
          {t.login.eyebrow}
        </span>
        <h1 className="m-0 font-display text-[28px] font-semibold leading-[1.15] tracking-[-0.02em] text-foreground">
          {t.login.title}
        </h1>

        <Button
          type="button"
          variant="outline"
          onClick={onGoogleSignIn}
          className="mt-2 h-[46px] w-full gap-2.5 text-[14.5px] font-medium"
          style={{ boxShadow: "var(--shadow-sm)" }}
        >
          <GoogleMark className="h-[18px] w-[18px]" />
          {t.login.googleSignIn}
        </Button>

        <div className="flex w-full items-center gap-3">
          <div className="h-px flex-1 bg-border" />
          <span className="text-[11px] text-muted-foreground">
            {t.login.orDivider}
          </span>
          <div className="h-px flex-1 bg-border" />
        </div>

        <Button
          type="button"
          variant="ghost"
          onClick={onEmailEntry}
          className="h-[42px] w-full text-[14px] font-medium text-foreground/85 hover:text-foreground"
        >
          {t.login.emailSignIn}
        </Button>
      </div>
    </section>
  );
}

/**
 * Email step 1 · collect the email and post request-code. On Sent →
 * step 2; on RateLimited → inline alert with countdown; on
 * network/5xx → bubble up to the top-level failure pane.
 */
function EmailStep1Pane({
  emailDraft,
  onEmailDraftChange,
  onBack,
  onSent,
  onReportFailure,
}: {
  emailDraft: string;
  onEmailDraftChange: (value: string) => void;
  onBack: () => void;
  onSent: (email: string) => void;
  onReportFailure: (failure: FailureKind) => void;
}) {
  const { t } = useTranslation();
  const [submitting, setSubmitting] = useState(false);
  // Local error reasons that should stay inline rather than escape to
  // the full failure pane (rate-limit countdown, bad email format).
  const [inlineError, setInlineError] = useState<string | null>(null);

  const submit = async () => {
    if (submitting) return;
    setInlineError(null);
    const trimmed = emailDraft.trim();
    if (!isPlausibleEmail(trimmed)) {
      setInlineError(t.login.email.stepEmail.invalid);
      return;
    }
    setSubmitting(true);
    try {
      const outcome = await authRequestEmailCode(trimmed);
      if (outcome.status === "sent") {
        onSent(trimmed.toLowerCase());
      } else {
        setInlineError(
          t.login.email.stepCode.errors.rateLimitedSend(
            outcome.retryAfterSeconds,
          ),
        );
      }
    } catch (error) {
      onReportFailure(classifyFailure(error));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <section className="flex flex-col items-start justify-center gap-4 bg-background px-14 py-12">
      <button
        type="button"
        onClick={onBack}
        className="flex items-center gap-1 text-[12px] text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="h-3.5 w-3.5" />
        {t.login.email.stepEmail.back}
      </button>
      <h2 className="m-0 max-w-[340px] font-display text-[22px] font-semibold leading-[1.3] tracking-[-0.015em] text-foreground">
        {t.login.email.stepEmail.title}
      </h2>
      <p className="max-w-[340px] text-[13px] leading-[1.55] text-muted-foreground">
        {t.login.email.stepEmail.sub}
      </p>

      <form
        className="mt-1 flex w-full max-w-[340px] flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <div className="flex flex-col gap-1.5">
          <Label
            htmlFor="login-email"
            className="text-[12px] font-medium text-foreground/80"
          >
            {t.login.email.stepEmail.label}
          </Label>
          <Input
            id="login-email"
            type="email"
            inputMode="email"
            autoComplete="email"
            autoCapitalize="off"
            autoCorrect="off"
            spellCheck={false}
            placeholder={t.login.email.stepEmail.placeholder}
            value={emailDraft}
            onChange={(e) => onEmailDraftChange(e.target.value)}
            disabled={submitting}
            autoFocus
          />
        </div>
        {inlineError ? (
          <p className="text-[12px] text-destructive">{inlineError}</p>
        ) : null}
        <Button
          type="submit"
          disabled={submitting || emailDraft.trim().length === 0}
          className="h-[42px] w-full"
        >
          {submitting
            ? t.login.email.stepEmail.sending
            : t.login.email.stepEmail.submit}
        </Button>
      </form>
    </section>
  );
}

/**
 * Email step 2 · collect the OTP code, verify, and complete login.
 * Owns its own 60s resend countdown — the timer starts on mount
 * because step 1 always lands here right after a successful send.
 */
function EmailStep2Pane({
  email,
  onChangeEmail,
  onLoggedIn,
  onReportFailure,
}: {
  email: string;
  onChangeEmail: () => void;
  onLoggedIn: (status: Parameters<typeof applyAuthStatus>[0]) => void;
  onReportFailure: (failure: FailureKind) => void;
}) {
  const { t } = useTranslation();
  const [code, setCode] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [resending, setResending] = useState(false);
  const [rejectCode, setRejectCode] = useState<EmailRejectCode | null>(null);
  const [resendInlineError, setResendInlineError] = useState<string | null>(null);
  const [cooldown, setCooldown] = useState(60);

  // Tick the cooldown counter down to zero so the "重发" button shows
  // an accurate timer. Refs are used so the interval owns its own
  // state and we don't accidentally double-register on rerenders.
  useEffect(() => {
    if (cooldown <= 0) return;
    const id = window.setInterval(() => {
      setCooldown((prev) => (prev > 0 ? prev - 1 : 0));
    }, 1000);
    return () => window.clearInterval(id);
  }, [cooldown]);

  const codeInputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    codeInputRef.current?.focus();
  }, []);

  const submit = async () => {
    if (submitting) return;
    if (!/^\d{6}$/.test(code)) {
      setRejectCode("invalid_code");
      return;
    }
    setSubmitting(true);
    setRejectCode(null);
    try {
      const outcome = await authLoginEmail(email, code);
      if (outcome.status === "ok") {
        onLoggedIn(outcome.auth);
      } else {
        setRejectCode(outcome.code);
        // Clear the input on hard-stop rejections so the user starts
        // fresh on "重发"; leave it alone for an in-place retry.
        if (
          outcome.code === "code_expired" ||
          outcome.code === "too_many_attempts"
        ) {
          setCode("");
        }
      }
    } catch (error) {
      onReportFailure(classifyFailure(error));
    } finally {
      setSubmitting(false);
    }
  };

  const resend = async () => {
    if (resending || cooldown > 0) return;
    setResending(true);
    setResendInlineError(null);
    setRejectCode(null);
    try {
      const outcome = await authRequestEmailCode(email);
      if (outcome.status === "sent") {
        setCooldown(60);
        setCode("");
        codeInputRef.current?.focus();
      } else {
        setCooldown(outcome.retryAfterSeconds);
        setResendInlineError(
          t.login.email.stepCode.errors.rateLimitedSend(
            outcome.retryAfterSeconds,
          ),
        );
      }
    } catch (error) {
      onReportFailure(classifyFailure(error));
    } finally {
      setResending(false);
    }
  };

  const inlineErrorMessage = (() => {
    if (resendInlineError) return resendInlineError;
    if (!rejectCode) return null;
    switch (rejectCode) {
      case "invalid_code":
        return t.login.email.stepCode.errors.invalidCode;
      case "code_expired":
        return t.login.email.stepCode.errors.codeExpired;
      case "too_many_attempts":
        return t.login.email.stepCode.errors.tooManyAttempts;
    }
  })();

  return (
    <section className="flex flex-col items-start justify-center gap-4 bg-background px-14 py-12">
      <button
        type="button"
        onClick={onChangeEmail}
        className="flex items-center gap-1 text-[12px] text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="h-3.5 w-3.5" />
        {t.login.email.stepCode.changeEmail}
      </button>
      <h2 className="m-0 max-w-[340px] font-display text-[22px] font-semibold leading-[1.3] tracking-[-0.015em] text-foreground">
        {t.login.email.stepCode.title}
      </h2>
      <p className="max-w-[340px] text-[13px] leading-[1.55] text-muted-foreground">
        {t.login.email.stepCode.sub(email)}
      </p>

      <form
        className="mt-1 flex w-full max-w-[340px] flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <div className="flex flex-col gap-1.5">
          <Label
            htmlFor="login-otp-code"
            className="text-[12px] font-medium text-foreground/80"
          >
            {t.login.email.stepCode.label}
          </Label>
          <Input
            ref={codeInputRef}
            id="login-otp-code"
            type="text"
            inputMode="numeric"
            autoComplete="one-time-code"
            pattern="\d{6}"
            maxLength={6}
            placeholder={t.login.email.stepCode.placeholder}
            value={code}
            onChange={(e) => {
              // Strip non-digits as the user types so paste-with-spaces
              // ("123 456") also works.
              const digits = e.target.value.replace(/\D/g, "").slice(0, 6);
              setCode(digits);
              if (rejectCode) setRejectCode(null);
            }}
            disabled={submitting}
            className="tracking-[0.4em] font-mono text-[16px]"
          />
        </div>
        {inlineErrorMessage ? (
          <p className="text-[12px] text-destructive">{inlineErrorMessage}</p>
        ) : null}
        <Button
          type="submit"
          disabled={submitting || code.length !== 6}
          className="h-[42px] w-full"
        >
          {submitting
            ? t.login.email.stepCode.verifying
            : t.login.email.stepCode.submit}
        </Button>
        <button
          type="button"
          onClick={() => void resend()}
          disabled={resending || cooldown > 0}
          className="text-[12px] text-muted-foreground enabled:hover:text-foreground disabled:cursor-not-allowed"
        >
          {resending
            ? t.login.email.stepCode.resending
            : cooldown > 0
              ? t.login.email.stepCode.resendIn(cooldown)
              : t.login.email.stepCode.resend}
        </button>
      </form>
    </section>
  );
}

/// Soft validation — the server is the actual authority. We just
/// rule out the obviously-wrong (no @, no dot) so we don't ping the
/// server for typos.
function isPlausibleEmail(email: string): boolean {
  return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email);
}

/**
 * Waiting-pane copy — replaces the stock spinner. The mascot breathes
 * (subtle scale loop) and Corivo "speaks" while the user is bouncing
 * around in the system browser. Stays fully readable; no animation
 * drama.
 */
function WaitingPane() {
  const { t } = useTranslation();
  return (
    <section className="flex flex-col items-start justify-center gap-4 bg-background px-14 py-12">
      <Mascot size="sm" breath />
      <h2
        className="m-0 max-w-[340px] whitespace-pre-line font-display text-[22px] font-semibold leading-[1.35] tracking-[-0.015em] text-foreground"
      >
        {t.login.waiting.title}
      </h2>
      <p className="max-w-[340px] text-[13px] leading-[1.55] text-muted-foreground">
        {t.login.waiting.sub}{" "}
        <a
          href="#"
          className="text-foreground/80 underline decoration-[color-mix(in_oklab,var(--foreground)_18%,transparent)] underline-offset-2 hover:decoration-foreground"
        >
          {t.login.waiting.manualLink}
        </a>
      </p>
      <span className="mt-1 cursor-pointer text-[12px] text-muted-foreground hover:text-foreground">
        {t.login.waiting.cancel}
      </span>
    </section>
  );
}

/**
 * Inline failure card — replaces the toast-driven error path. Each
 * branch has its own copy + the same primary "Try again" button. The
 * `denied` (closed-beta whitelist) branch gets a custom layout that
 * surfaces the rejected email so the user knows which account to
 * change.
 */
function FailurePane({
  failure,
  onRetry,
}: {
  failure: FailureKind;
  onRetry: () => void;
}) {
  const { t } = useTranslation();

  if (failure.kind === "denied") {
    return (
      <section className="flex flex-col items-start justify-center gap-4 bg-background px-14 py-12">
        <ErrorMark />
        <h2 className="m-0 max-w-[340px] font-display text-[22px] font-semibold leading-[1.3] tracking-[-0.015em] text-foreground">
          {t.login.failure.deniedTitle}
        </h2>
        {failure.error.message ? (
          <p className="max-w-[340px] text-[13px] leading-[1.55] text-muted-foreground">
            {failure.error.message}
          </p>
        ) : null}
        {failure.error.email ? (
          <p className="text-[12px] text-muted-foreground">
            {t.login.failure.deniedAccountLabel}{" "}
            <span className="font-mono text-foreground">
              {failure.error.email}
            </span>
          </p>
        ) : null}
        <p className="max-w-[340px] text-[12px] text-muted-foreground">
          {t.login.failure.deniedHint}
        </p>
        <div className="mt-2 flex gap-2.5">
          <Button onClick={onRetry} className="gap-2">
            {t.login.failure.retry}
            <ArrowRight className="h-3.5 w-3.5" />
          </Button>
        </div>
      </section>
    );
  }

  const { title, desc } = (() => {
    switch (failure.kind) {
      case "network":
        return {
          title: t.login.failure.networkTitle,
          desc: t.login.failure.networkDesc,
        };
      case "timeout":
        return {
          title: t.login.failure.timeoutTitle,
          desc: t.login.failure.timeoutDesc,
        };
      case "cancelled":
        return {
          title: t.login.failure.cancelledTitle,
          desc: t.login.failure.cancelledDesc,
        };
      default:
        return {
          title: t.login.failure.genericTitle,
          desc: failure.message,
        };
    }
  })();

  return (
    <section className="flex flex-col items-start justify-center gap-4 bg-background px-14 py-12">
      <ErrorMark />
      <h2 className="m-0 max-w-[340px] font-display text-[22px] font-semibold leading-[1.3] tracking-[-0.015em] text-foreground">
        {title}
      </h2>
      <p className="max-w-[340px] text-[13px] leading-[1.55] text-muted-foreground">
        {desc}
      </p>
      <div className="mt-2 flex gap-2.5">
        <Button onClick={onRetry} className="gap-2">
          {t.login.failure.retry}
          <ArrowRight className="h-3.5 w-3.5" />
        </Button>
      </div>
    </section>
  );
}

function ErrorMark() {
  return (
    <div className="flex h-9 w-9 items-center justify-center rounded-full bg-destructive/10 text-destructive">
      <svg
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth={1.5}
        strokeLinecap="round"
        strokeLinejoin="round"
        className="h-4 w-4"
        aria-hidden="true"
      >
        <path d="M1 1l22 22" />
        <path d="M16.72 11.06A10.94 10.94 0 0 1 19 12.55" />
        <path d="M5 12.55a10.94 10.94 0 0 1 5.17-2.39" />
        <path d="M10.71 5.05A16 16 0 0 1 22.58 9" />
        <path d="M1.42 9a15.91 15.91 0 0 1 4.7-2.88" />
        <path d="M8.53 16.11a6 6 0 0 1 6.95 0" />
        <path d="M12 20h.01" />
      </svg>
    </div>
  );
}

// Inline SVG of the multi-color Google "G" mark. Keeping it inline
// avoids a network fetch on the login page (no internet → blank
// button) and dodges the licensing wrinkles around redistributing the
// PNG; the path data is a recreation per Google's brand guidelines
// for sign-in buttons.
function GoogleMark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className} aria-hidden="true" role="img">
      <path
        fill="#4285F4"
        d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92c-.26 1.37-1.04 2.53-2.21 3.31v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.09z"
      />
      <path
        fill="#34A853"
        d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z"
      />
      <path
        fill="#FBBC05"
        d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09 0-.73.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z"
      />
      <path
        fill="#EA4335"
        d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z"
      />
    </svg>
  );
}
