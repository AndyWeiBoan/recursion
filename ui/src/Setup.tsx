// First-run setup and the pieces it shares with Settings › 帳號: install the CLIs (their official
// installers, run in a terminal shown here), then add logins (one the CLI already has on this
// computer, or a new one through the CLI's own login).
import { useEffect, useRef, useState } from "react";
import { t } from "./i18n";
import { detectLogins, send, setSetup, useAccounts, useAccountsLoaded, useSetup, type CliInfo } from "./client";
import { disposeTerminal, fitTerminal, focusTerminal, mountTerminal } from "./terminals";
import { Modal } from "./WorkspaceView";
import type { Account, AgentKind } from "./types";
import { Icon } from "./icons";

export const KINDS: { kind: AgentKind; name: string }[] = [
  { kind: "claude", name: "Claude Code" },
  { kind: "codex", name: "Codex" },
];

/** What to call an account: its name, else its email, else "new account". */
export function accountLabel(a: Account) {
  return a.name || a.email || t("acct.newAccount");
}

/** Email and plan, leaving out the email when it already is the name. */
export function accountDetail(a: Account) {
  return [a.email !== accountLabel(a) ? a.email : null, a.plan].filter(Boolean).join(" · ");
}

/** The terminal a CLI installer or login runs in, over everything else. */
export function SetupTerminal() {
  const { panel, cli } = useSetup();
  const accounts = useAccounts();
  const host = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!panel || !host.current) return;
    mountTerminal(panel.id, host.current);
    const ro = new ResizeObserver(() => fitTerminal(panel.id));
    ro.observe(host.current);
    setTimeout(() => focusTerminal(panel.id), 50);
    return () => ro.disconnect();
  }, [panel?.id]);
  const login = panel?.id.startsWith("setup-login-") ? accounts.find((a) => a.id === panel.id.slice("setup-login-".length)) : undefined;
  const installKind = panel?.id.startsWith("setup-install-") ? (panel.id.slice("setup-install-".length) as AgentKind) : undefined;
  const reached = login ? login.logged_in : installKind ? !!cli?.[installKind] : false;
  // Only a change while open counts: logging in again starts out logged in.
  const atOpen = useRef<{ id: string; reached: boolean } | null>(null);
  if (panel && atOpen.current?.id !== panel.id) atOpen.current = { id: panel.id, reached };
  const done = reached && !atOpen.current?.reached;
  const close = () => {
    if (!panel) return;
    send({ t: "setup_close", panel: panel.id });
    disposeTerminal(panel.id);
    setSetup({ panel: null });
    if (installKind) send({ t: "cli_status", fresh: true });
    detectLogins();
  };
  // Finished: show the ✓ a moment, then get out of the way.
  const closeRef = useRef(close);
  closeRef.current = close;
  useEffect(() => {
    if (!done) return;
    const id = setTimeout(() => closeRef.current(), 1500);
    return () => clearTimeout(id);
  }, [done, panel?.id]);
  if (!panel) return null;
  return (
    <Modal title={panel.title} onClose={close} escape={false}>
      <div className="setup-term">
        <h3>{panel.title}</h3>
        <p className={done ? "setup-ok" : "dim"}>
          {done ? (login ? t("setup.loginDone", { email: login.email ?? "" }) : t("setup.installDone")) : login ? t(login.kind === "claude" ? "setup.loginHintClaude" : "setup.loginHint") : t("setup.installHint")}
        </p>
        <div className="setup-xterm" ref={host} />
        <div className="setup-foot">
          <button className={done ? "primary" : ""} onClick={close}>
            {done ? t("setup.finish") : t("common.close")}
          </button>
        </div>
      </div>
    </Modal>
  );
}

function CliRow({ name, kind, info }: { name: string; kind: AgentKind; info: CliInfo | null | undefined }) {
  return (
    <div className="setup-row">
      <span className={`setup-mark ${info ? "ok" : info === null ? "no" : ""}`}>{info ? <Icon name="check" size={13} /> : info === null ? <Icon name="close" size={13} /> : "…"}</span>
      <div className="setup-main">
        <b>{name}</b>
        <span className="dim">{info ? t("setup.installed", { version: info.version || "?" }) : info === null ? t("setup.notInstalled") : t("setup.checking")}</span>
      </div>
      {info === null && (
        <button className="primary" onClick={() => send({ t: "cli_install", kind })}>
          {t("setup.install")}
        </button>
      )}
    </div>
  );
}

/** Ways to add an account of one kind: the login already on this computer, or a new login. */
export function AddAccountChoices({ kind }: { kind: AgentKind }) {
  const { logins } = useSetup();
  const found = logins?.find((l) => l.kind === kind);
  const has = useAccounts().some((a) => a.kind === kind);
  // Wait for the answer: a login button shown meanwhile gets clicked, and starts a new login
  // in whatever account the browser is on instead of using the one already here.
  if (logins === null)
    return (
      <div className="setup-row">
        <span className="setup-mark">…</span>
        <div className="setup-main">
          <span className="dim">{t("setup.lookingForLogins")}</span>
        </div>
      </div>
    );
  return (
    <div className="setup-add">
      {found && (
        <div className="setup-row found">
          <span className="setup-mark ok"><Icon name="check" size={13} /></span>
          <div className="setup-main">
            <b>{found.email ?? t("acct.loggedIn")}</b>
            <span className="dim">{t("setup.foundLogin")}{found.plan ? ` · ${found.plan}` : ""}</span>
          </div>
          <button className="primary" onClick={() => (send({ t: "account_adopt", kind }), setSetup({ logins: (logins ?? []).filter((l) => l !== found) }))}>
            {t("setup.addThis")}
          </button>
        </div>
      )}
      <button className="setup-new" onClick={() => send({ t: "account_create", kind })}>
        {found || has ? t("setup.loginOther") : t("setup.loginNew")}
      </button>
    </div>
  );
}

function AccountRows({ kind }: { kind: AgentKind }) {
  const accounts = useAccounts().filter((a) => a.kind === kind);
  return (
    <>
      {accounts.map((a) => (
        <div key={a.id} className="setup-row">
          <span className={`setup-mark ${a.logged_in ? "ok" : ""}`}>{a.logged_in ? <Icon name="check" size={13} /> : "…"}</span>
          <div className="setup-main">
            <b>{accountLabel(a)}</b>
            <span className="dim">{a.logged_in ? accountDetail(a) || t("acct.loggedIn") : t("setup.waitingLogin")}</span>
          </div>
          {!a.logged_in && <button onClick={() => send({ t: "account_login", account: a.id })}>{t("acct.login")}</button>}
        </div>
      ))}
    </>
  );
}

/** First run: shown at start while no account is logged in. */
export function Onboarding() {
  const loaded = useAccountsLoaded();
  const accounts = useAccounts();
  const { cli, panel } = useSetup();
  const [step, setStep] = useState(0);
  const [closed, setClosed] = useState(false);
  // Decided once, when the account list first arrives: adding accounts must not end setup midway.
  const [show, setShow] = useState<boolean | null>(null);
  useEffect(() => {
    if (loaded && show === null) setShow(!accounts.some((a) => a.logged_in));
  }, [loaded]);
  useEffect(() => {
    if (show) send({ t: "cli_status", fresh: true });
  }, [show]);
  useEffect(() => {
    if (show && step === 1) detectLogins();
  }, [show, step]);
  if (!show || closed) return null;
  const installed = KINDS.filter(({ kind }) => cli?.[kind]);
  const ready = accounts.some((a) => a.logged_in);
  return (
    <Modal title={t("setup.title")} onClose={() => setClosed(true)} escape={!panel}>
      <div className="setup">
        <ol className="setup-steps">
          {[t("setup.step.tools"), t("setup.step.accounts")].map((s, i) => (
            <li key={s} className={i === step ? "on" : i < step ? "done" : ""}>
              <span>{i < step ? <Icon name="check" size={12} /> : i + 1}</span>
              {s}
            </li>
          ))}
        </ol>
        {step === 0 && (
          <>
            <h3>{t("setup.tools.title")}</h3>
            <p className="dim">{t("setup.tools.hint")}</p>
            {KINDS.map(({ kind, name }) => (
              <CliRow key={kind} kind={kind} name={name} info={cli ? cli[kind] : undefined} />
            ))}
            <div className="setup-foot">
              <button onClick={() => send({ t: "cli_status", fresh: true })}>{t("setup.recheck")}</button>
              <button className="primary" disabled={!installed.length} onClick={() => setStep(1)}>
                {t("setup.next")}
              </button>
            </div>
          </>
        )}
        {step === 1 && (
          <>
            <h3>{t("setup.accounts.title")}</h3>
            <p className="dim">{t("setup.accounts.hint")}</p>
            {installed.map(({ kind, name }) => (
              <section key={kind} className="setup-group">
                <h4>{name}</h4>
                <AccountRows kind={kind} />
                <AddAccountChoices kind={kind} />
              </section>
            ))}
            <div className="setup-foot">
              <button onClick={() => setStep(0)}>{t("setup.back")}</button>
              <button className="primary" disabled={!ready} onClick={() => setClosed(true)}>
                {t("setup.start")}
              </button>
            </div>
          </>
        )}
      </div>
    </Modal>
  );
}
