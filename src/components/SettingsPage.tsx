import {
  type ReactNode,
  useContext,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { ACCENTS, DEFAULT_ACCENT } from "../avatars/catalog";
import {
  applyAppearance,
  prefersDark,
  resolveSurface,
  watchSystemSurface,
} from "../lib/appearance";
import { buildLabel } from "../lib/build";
import { dismissing, useReturnFocus } from "../lib/dismiss";
import { clientKeeps, hostAt, hostPlace } from "../lib/host";
import { api, openExternal } from "../lib/ipc";
import { BINDINGS, comboLabel, SURFACES } from "../lib/keybinds";
import { LIMITS } from "../lib/limits";
import {
  CASTS,
  type Cast,
  type Prefs,
  READING_SIZES,
  READINGS,
  type Reading,
  type SurfaceMode,
  TYPEFACES,
  type Typeface,
  UI_SCALES,
} from "../lib/prefs";
import { type Provider as Preset, planLabel, providerFor } from "../lib/providers";
import { SeasonContext } from "../lib/season";
import { SECTION_LABELS, SECTIONS, type Section } from "../lib/sections";
import { type SettingsDraft, useStore } from "../lib/store";
import { moveAlongTabs } from "../lib/tablist";
import {
  ATTENTIONS,
  type AttentionKey,
  CONTRASTS,
  type ContrastLevel,
  GRAYS,
  type GrayKey,
  matchesPreset,
  TEMPERATURE_KEYS,
  TEMPERATURES,
  THEME_PRESETS,
  themeTokens,
} from "../lib/theme";
import { whenLabel } from "../lib/time";
import { attached, desktop } from "../lib/transport";
import {
  type AccountConnectors,
  type AccountStatus,
  type Attention,
  type DeliveryPatch,
  type DeviceCode,
  errorMessage,
  type GuardLimits,
  type NotificationAccess,
  type Provider as ProviderKind,
  type PushContent,
  type PushFormat,
  type PushReport,
  type Settings,
  type SettingsPatch,
  type SubscriptionStatus,
} from "../lib/types";
import { ClaudeSignin } from "./ClaudeSignin";
import { Compost } from "./Compost";
import { GroupTransfer, LegacyGroups } from "./GroupTransfer";
import { HostChoice } from "./HostSetup";
import { HostUpdatePanel } from "./HostUpdates";
import { Permissions } from "./Permissions";
import { ProviderPresets, SubscriptionModel } from "./ProviderFields";
import { SkillList } from "./SkillList";

interface Props {
  request: { section: Section | null };
  onSection?: (section: Section) => void;
  notices?: ReactNode;
}

function SettingChoice<T>({
  name,
  title,
  hint,
  value,
  options,
  list,
  busy,
  onChange,
}: {
  name: string;
  title?: string;
  hint?: string;
  value: T | undefined;
  options: readonly { value: T; label: string; name?: string; preview?: ReactNode }[];
  list?: boolean;
  busy?: boolean;
  onChange: (value: T) => void;
}) {
  const id = useId();
  return (
    <div className="setting-choice" data-layout={list || options.length > 4 ? "list" : "segments"}>
      <div>
        <span className="field__label" id={id} title={title}>
          {name}
        </span>
        {hint && <p className="field__hint">{hint}</p>}
      </div>
      <div className="setting-choice__options" role="radiogroup" aria-labelledby={id}>
        {options.map((option) => (
          <label className="setting-choice__option" key={option.label}>
            <input
              type="radio"
              name={id}
              aria-label={option.name ?? `${name}: ${option.label}`}
              checked={value === option.value}
              aria-disabled={busy || undefined}
              onChange={() => {
                if (!busy) onChange(option.value);
              }}
            />
            <span className="setting-choice__label">
              {option.preview && (
                <span className="setting-choice__preview" aria-hidden="true">
                  {option.preview}
                </span>
              )}
              {option.label}
            </span>
          </label>
        ))}
      </div>
    </div>
  );
}

const SURFACE_LABELS: Record<SurfaceMode, string> = {
  light: "Light",
  dark: "Dark",
  system: "Match system",
};

const TYPEFACE_LABELS: Record<Typeface, string> = {
  inter: "Inter",
  system: "System",
  legible: "Atkinson Hyperlegible",
};

const READING_LABELS: Record<Reading, string> = {
  interface: "Same as the interface",
  serif: "Literata",
};

const CONTRAST_LABELS: Record<ContrastLevel, string> = {
  standard: "Standard",
  more: "More",
};

const CAST_LABELS: Record<Cast, string> = {
  cut: "Cut",
  drawn: "Drawn",
};

const CAST_SAMPLE = [
  { avatar: "orb", color: "Olive" },
  { avatar: "slab", color: "Slate" },
  { avatar: "lobe", color: "Terracotta" },
].map((one) => ({
  avatar: one.avatar,
  color: ACCENTS.find((accent) => accent.name === one.color)?.value ?? DEFAULT_ACCENT,
}));

const ATTENTION_ROWS: { key: keyof Attention; label: string; hint: string }[] = [
  {
    key: "needsYou",
    label: "Things that need you",
    hint: "A question, permission included, an agent that is stuck, and reminders about open questions in For you.",
  },
  {
    key: "failures",
    label: "Failures",
    hint: "A conversation that failed, or was cut off when the Host restarted.",
  },
  {
    key: "finished",
    label: "Finished work while you are away",
    hint: "Collected into one notification each time you step away. Nothing is sent while you are at a window.",
  },
];

const NOT_ACCEPTED = "Not sent. macOS did not accept the notification.";

const TEST_NOTIFICATION = "This is what a notification looks like.";

const MAC_ACCESS: Record<NotificationAccess, string> = {
  allowed: "macOS allows Guaca to show notifications.",
  notAsked:
    "macOS has not asked you about Guaca yet. It asks the first time Guaca has something to show, or when you send a test.",
  denied: "macOS has notifications turned off for Guaca.",
  unavailable:
    "This copy of Guaca is not running as an installed app, so macOS will not show its notifications.",
};

const PUSH_FORMATS: { value: PushFormat; label: string }[] = [
  { value: "ntfy", label: "ntfy" },
  { value: "json", label: "JSON webhook" },
];

const PUSH_CONTENTS: { value: PushContent; label: string }[] = [
  { value: "outcome", label: "Include what finished" },
  { value: "generic", label: "Only say that something finished" },
];

function said(at: number, now: number): string {
  return whenLabel(at, now).replace(/^(Today|Yesterday)/, (day) => day.toLowerCase());
}

export function pushLine(
  report: PushReport | null,
  test: { ok: boolean; text: string } | null,
  now: number,
): { tone: "ok" | "error" | "info"; text: string } | null {
  if (test) return { tone: test.ok ? "ok" : "error", text: test.text };
  if (!report) return null;
  if (report.lastError && report.retryAt !== null)
    return {
      tone: "error",
      text: `Last attempt failed: ${report.lastError} Trying again ${said(report.retryAt, now)}.`,
    };
  if (report.lastError) return { tone: "error", text: `Last attempt failed: ${report.lastError}` };
  if (report.lastSentAt !== null)
    return { tone: "info", text: `Last sent ${said(report.lastSentAt, now)}` };
  return { tone: "info", text: "Nothing sent yet." };
}

const differs = (text: string, stored: number | undefined) =>
  text.trim() !== "" && Number(text) !== stored;

export function SettingsPage({ request, onSection, notices }: Props) {
  const opening = useRef(request).current.section ?? undefined;
  const initial = useRef(useStore.getState().settingsDraft).current;
  const settings = useStore((s) => s.settings);
  const setSettings = useStore((s) => s.setSettings);
  const capabilities = useStore((s) => s.capabilities);
  const prefs = useStore((s) => s.prefs);
  const [systemDark, setSystemDark] = useState(prefersDark);
  useEffect(() => watchSystemSurface(setSystemDark), []);
  const setPrefs = useStore((s) => s.setPrefs);
  const place = hostPlace();
  const keeps = clientKeeps({ desktop, attached: attached() !== null });
  const season = useContext(SeasonContext);
  const setLook = (patch: Partial<Prefs>) => {
    setPrefs(patch);
    applyAppearance({ ...prefs, ...patch, season });
  };

  const [section, setSection] = useState<Section>(opening ?? "general");
  useEffect(() => {
    if (request.section) setSection(request.section);
  }, [request]);
  const [visitedPermissions, setVisitedPermissions] = useState(opening === "permissions");
  useEffect(() => {
    if (section === "permissions") setVisitedPermissions(true);
  }, [section]);
  useEffect(() => onSection?.(section), [section, onSection]);
  const [operatorName, setOperatorName] = useState(
    initial?.operatorName ?? settings?.operatorName ?? "",
  );
  const [provider, setProvider] = useState<ProviderKind>(
    initial?.provider ?? settings?.provider ?? "compatible",
  );
  const [baseUrl, setBaseUrl] = useState(initial?.baseUrl ?? settings?.baseUrl ?? "");
  const [model, setModel] = useState(initial?.model ?? settings?.defaultModel ?? "");
  const [reasoningEffort, setReasoningEffort] = useState<import("../lib/types").ReasoningEffort>(
    initial?.reasoningEffort ?? settings?.reasoningEffort ?? "auto",
  );
  const [subscriptionModel, setSubscriptionModel] = useState(
    initial?.subscriptionModel ?? settings?.subscriptionModel ?? "",
  );
  const [apiKey, setApiKey] = useState("");
  const [editingApiKey, setEditingApiKey] = useState(false);
  const [e2bKey, setE2bKey] = useState("");
  const [editingE2bKey, setEditingE2bKey] = useState(false);
  const [idleMinutes, setIdleMinutes] = useState(initial?.idleMinutes ?? "");
  const [kernelKey, setKernelKey] = useState("");
  const [editingKernelKey, setEditingKernelKey] = useState(false);
  const [browserIdleMinutes, setBrowserIdleMinutes] = useState(initial?.browserIdleMinutes ?? "");
  const [stealth, setStealth] = useState(initial?.stealth ?? settings?.browserStealth ?? false);
  const [timeout, setTimeoutSecs] = useState(initial?.timeout ?? "");
  const [limits, setLimits] = useState<GuardLimits>(
    initial?.limits ??
      settings?.limits ?? {
        maxHops: 8,
        maxStepsPerRun: 60,
        maxFanoutPerCall: 8,
        maxSendsPerPair: 6,
        maxToolRounds: 24,
      },
  );
  const [status, setStatus] = useState<{ tone: "ok" | "error" | "info"; text: string } | null>(
    null,
  );
  const [busy, setBusy] = useState(false);
  const [macAccess, setMacAccess] = useState<NotificationAccess | null>(null);
  const [attending, setAttending] = useState<keyof Attention | null>(null);
  const [pushUrl, setPushUrl] = useState("");
  const [pushToken, setPushToken] = useState("");
  const [pushLink, setPushLink] = useState<string | null>(null);
  const [pushBusy, setPushBusy] = useState(false);
  const [pushReport, setPushReport] = useState<PushReport | null>(null);
  const [pushTest, setPushTest] = useState<{ ok: boolean; text: string } | null>(null);
  const panelRef = useRef<HTMLElement>(null);
  const navRef = useRef<HTMLDivElement>(null);
  const paneRef = useRef<HTMLDivElement>(null);
  const pending = useRef<SettingsPatch | null>(null);

  const base = useRef(initial?.baseline ?? settings);
  useEffect(() => {
    const was = base.current;
    base.current = settings;
    if (!settings || !was || was === settings) return;
    const inFlight = pending.current;
    const follow =
      <T,>(before: T, after: T) =>
      (mine: T) =>
        mine === before ? after : mine;
    if (inFlight?.operatorName === undefined)
      setOperatorName(follow(was.operatorName, settings.operatorName));
    if (inFlight?.provider === undefined) setProvider(follow(was.provider, settings.provider));
    if (inFlight?.baseUrl === undefined) setBaseUrl(follow(was.baseUrl, settings.baseUrl));
    if (inFlight?.defaultModel === undefined)
      setModel(follow(was.defaultModel, settings.defaultModel));
    if (inFlight?.reasoningEffort === undefined)
      setReasoningEffort(follow(was.reasoningEffort, settings.reasoningEffort));
    if (inFlight?.subscriptionModel === undefined)
      setSubscriptionModel(follow(was.subscriptionModel, settings.subscriptionModel));
    if (inFlight?.browserStealth === undefined)
      setStealth(follow(was.browserStealth, settings.browserStealth));
    setLimits((mine) => {
      const next = { ...mine };
      for (const { key } of LIMITS)
        if (inFlight?.limits?.[key] === undefined)
          next[key] = follow(was.limits[key], settings.limits[key])(mine[key]);
      return next;
    });
  }, [settings]);
  useEffect(() => {
    useStore.setState({ settingsDraft: null });
  }, []);

  const currentKeys = useRef({ apiKey, e2bKey, kernelKey });
  currentKeys.current = { apiKey, e2bKey, kernelKey };

  const [subscription, setSubscription] = useState<SubscriptionStatus | null>(null);
  const [pendingCode, setPendingCode] = useState<DeviceCode | null>(null);

  const [claudeInstalled, setClaudeInstalled] = useState<boolean | null>(null);
  const [account, setAccount] = useState<AccountStatus | null>(null);
  const [connectors, setConnectors] = useState<AccountConnectors | null>(null);
  const [linking, setLinking] = useState(false);

  useEffect(() => {
    panelRef.current?.focus();
  }, []);

  useLayoutEffect(() => {
    const regions = [navRef.current, paneRef.current].filter((node) => node !== null);
    const measure = () => {
      for (const node of regions) {
        const gutter =
          node.offsetWidth -
          node.clientWidth -
          node.clientLeft -
          parseFloat(getComputedStyle(node).borderRightWidth);
        node.style.setProperty("--settings-gutter", `${Math.max(0, gutter)}px`);
      }
    };
    measure();
    const observer = new ResizeObserver(measure);
    for (const node of regions) observer.observe(node);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    let live = true;
    void api
      .claudeProgram()
      .then((program) => {
        if (live) setClaudeInstalled(program.installed);
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);

  const patch = (): SettingsPatch => ({
    provider,
    baseUrl,
    defaultModel: model,
    reasoningEffort,
    ...(subscriptionModel.trim() ? { subscriptionModel: subscriptionModel.trim() } : {}),
    ...(apiKey.trim() ? { apiKey: apiKey.trim() } : {}),
    ...(e2bKey.trim() ? { e2bApiKey: e2bKey.trim() } : {}),
    ...(idleMinutes.trim() ? { computerIdleMinutes: Number(idleMinutes) } : {}),
    ...(kernelKey.trim() ? { kernelApiKey: kernelKey.trim() } : {}),
    ...(browserIdleMinutes.trim() ? { browserIdleMinutes: Number(browserIdleMinutes) } : {}),
    browserStealth: stealth,
    ...(timeout.trim() ? { requestTimeoutSecs: Number(timeout) } : {}),
  });

  const dirty =
    operatorName !== (settings?.operatorName ?? "") ||
    provider !== (settings?.provider ?? "compatible") ||
    baseUrl !== (settings?.baseUrl ?? "") ||
    model !== (settings?.defaultModel ?? "") ||
    subscriptionModel !== (settings?.subscriptionModel ?? "") ||
    reasoningEffort !== (settings?.reasoningEffort ?? "auto") ||
    stealth !== (settings?.browserStealth ?? false) ||
    apiKey.trim() !== "" ||
    e2bKey.trim() !== "" ||
    kernelKey.trim() !== "" ||
    differs(idleMinutes, settings?.computerIdleMinutes) ||
    differs(browserIdleMinutes, settings?.browserIdleMinutes) ||
    differs(timeout, settings?.requestTimeoutSecs) ||
    LIMITS.some((field) => limits[field.key] !== settings?.limits[field.key]);

  const changes = (): SettingsPatch => {
    const changedLimits = Object.fromEntries(
      LIMITS.filter(({ key }) => limits[key] !== settings?.limits[key]).map(({ key }) => [
        key,
        limits[key],
      ]),
    );
    const all: SettingsPatch = { operatorName, limits: changedLimits, ...patch() };
    if (!settings) return all;
    const now: Partial<Record<keyof SettingsPatch, unknown>> = {
      operatorName: settings.operatorName,
      provider: settings.provider,
      baseUrl: settings.baseUrl,
      defaultModel: settings.defaultModel,
      reasoningEffort: settings.reasoningEffort,
      subscriptionModel: settings.subscriptionModel,
      browserStealth: settings.browserStealth,
    };
    const sent = Object.fromEntries(
      Object.entries(all).filter(([key, value]) =>
        key === "limits"
          ? LIMITS.some((field) => limits[field.key] !== settings.limits[field.key])
          : !(key in now) || now[key as keyof SettingsPatch] !== value,
      ),
    );
    return sent as SettingsPatch;
  };

  useEffect(() => {
    if (section !== "notifications" || !desktop) return;
    let live = true;
    const read = () =>
      api.notificationAccess().then(
        (access) => live && setMacAccess(access),
        () => {},
      );
    void read();
    window.addEventListener("focus", read);
    return () => {
      live = false;
      window.removeEventListener("focus", read);
    };
  }, [section]);

  const pushSaved = settings?.delivery?.urlSet ?? false;
  useEffect(() => {
    if (section !== "notifications" || !pushSaved) return;
    let live = true;
    api.pushReport().then(
      (report) => live && setPushReport(report),
      () => {},
    );
    return () => {
      live = false;
    };
  }, [section, pushSaved, settings?.delivery]);

  const writePush = async (delivery: DeliveryPatch) => {
    setPushBusy(true);
    setStatus(null);
    try {
      setSettings(await api.updateSettings({ delivery }));
      setPushTest(null);
      return true;
    } catch (error) {
      setStatus({ tone: "error", text: errorMessage(error) });
      return false;
    } finally {
      setPushBusy(false);
    }
  };

  const pushTyped = (): DeliveryPatch => {
    const delivery: DeliveryPatch = {};
    if (pushUrl.trim()) delivery.url = pushUrl.trim();
    if (pushToken.trim()) delivery.token = pushToken.trim();
    if (pushLink !== null && pushLink.trim() !== (settings?.delivery?.linkBase ?? ""))
      delivery.linkBase = pushLink.trim();
    return delivery;
  };

  const savePush = async () => {
    if (!(await writePush(pushTyped()))) return;
    setPushUrl("");
    setPushToken("");
    setPushLink(null);
  };

  const testPush = async () => {
    setPushBusy(true);
    setPushTest(null);
    try {
      setPushTest({ ok: true, text: await api.testPush() });
    } catch (error) {
      setPushTest({ ok: false, text: `The test failed: ${errorMessage(error)}` });
    } finally {
      setPushBusy(false);
    }
  };

  const toggleAttention = async (key: keyof Attention) => {
    const now = settings?.attention;
    if (!now) return;
    setAttending(key);
    setStatus(null);
    try {
      setSettings(await api.updateSettings({ attention: { [key]: !now[key] } }));
    } catch (error) {
      setStatus({ tone: "error", text: errorMessage(error) });
    } finally {
      setAttending(null);
    }
  };

  const save = async () => {
    setBusy(true);
    setStatus(null);
    const sent = changes();
    pending.current = sent;
    try {
      let next: Settings;
      try {
        next = await api.updateSettings(sent);
      } catch (error) {
        const refusal = error as { kind?: unknown; message?: unknown } | null;
        if (
          refusal?.kind !== "badArguments" ||
          !sent.limits ||
          typeof refusal.message !== "string" ||
          !LIMITS.some(
            ({ key }) =>
              sent.limits?.[key] === undefined &&
              (refusal.message as string).startsWith(
                `\`update_settings\` was called with arguments this build does not recognize (missing field \`${key}\`)`,
              ),
          )
        ) {
          throw error;
        }
        const latest = await api.getSettings();
        next = await api.updateSettings({ ...sent, limits: { ...latest.limits, ...sent.limits } });
      }
      if (base.current) {
        base.current = {
          ...base.current,
          operatorName: sent.operatorName ?? base.current.operatorName,
          provider: sent.provider ?? base.current.provider,
          baseUrl: sent.baseUrl ?? base.current.baseUrl,
          defaultModel: sent.defaultModel ?? base.current.defaultModel,
          reasoningEffort: sent.reasoningEffort ?? base.current.reasoningEffort,
          subscriptionModel: sent.subscriptionModel ?? base.current.subscriptionModel,
          browserStealth: sent.browserStealth ?? base.current.browserStealth,
          limits: { ...base.current.limits, ...sent.limits },
        };
      }
      pending.current = null;
      setSettings(next);
      setLimits(
        (current) =>
          Object.fromEntries(
            LIMITS.map(({ key }) => [
              key,
              current[key] === limits[key] ? next.limits[key] : current[key],
            ]),
          ) as unknown as GuardLimits,
      );
      setApiKey((value) => (value === apiKey ? "" : value));
      if (apiKey && currentKeys.current.apiKey === apiKey) setEditingApiKey(false);
      setE2bKey((value) => (value === e2bKey ? "" : value));
      setKernelKey((value) => (value === kernelKey ? "" : value));
      if (e2bKey && currentKeys.current.e2bKey === e2bKey) setEditingE2bKey(false);
      if (kernelKey && currentKeys.current.kernelKey === kernelKey) setEditingKernelKey(false);
      setIdleMinutes((value) => (value === idleMinutes ? "" : value));
      setBrowserIdleMinutes((value) => (value === browserIdleMinutes ? "" : value));
      setTimeoutSecs((value) => (value === timeout ? "" : value));
      setStatus({ tone: "ok", text: "Saved." });
    } catch (error) {
      setStatus({ tone: "error", text: errorMessage(error) });
    } finally {
      pending.current = null;
      setBusy(false);
    }
  };

  const kept = useRef<SettingsDraft | null>(null);
  kept.current = dirty
    ? {
        baseline: settings,
        operatorName,
        provider,
        baseUrl,
        model,
        reasoningEffort,
        subscriptionModel,
        idleMinutes,
        browserIdleMinutes,
        stealth,
        timeout,
        limits,
      }
    : null;
  useEffect(
    () => () => {
      useStore.setState({ settingsDraft: kept.current });
    },
    [],
  );

  const closeSettings = useStore((s) => s.closeSettings);
  const titleId = useId();
  useReturnFocus(() => useStore.getState().settingsGoneBack);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.isComposing || event.defaultPrevented || dismissing())
        return;
      event.preventDefault();
      closeSettings();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [closeSettings]);

  const discard = () => {
    setOperatorName(settings?.operatorName ?? "");
    setProvider(settings?.provider ?? "compatible");
    setBaseUrl(settings?.baseUrl ?? "");
    setModel(settings?.defaultModel ?? "");
    setReasoningEffort(settings?.reasoningEffort ?? "auto");
    setSubscriptionModel(settings?.subscriptionModel ?? "");
    setStealth(settings?.browserStealth ?? false);
    setLimits(
      settings?.limits ?? {
        maxHops: 8,
        maxStepsPerRun: 60,
        maxFanoutPerCall: 8,
        maxSendsPerPair: 6,
        maxToolRounds: 24,
      },
    );
    setApiKey("");
    setE2bKey("");
    setKernelKey("");
    setEditingApiKey(false);
    setEditingE2bKey(false);
    setEditingKernelKey(false);
    setIdleMinutes("");
    setBrowserIdleMinutes("");
    setTimeoutSecs("");
    setStatus(null);
  };

  const test = async () => {
    setBusy(true);
    setStatus({ tone: "info", text: "Testing…" });
    try {
      const result = await api.testConnection(patch());
      setStatus({ tone: "ok", text: result });
    } catch (error) {
      setStatus({ tone: "error", text: errorMessage(error) });
    } finally {
      setBusy(false);
    }
  };

  const choose = (preset: Preset) => {
    setProvider("compatible");
    setBaseUrl(preset.baseUrl);
    if (preset.baseUrl !== baseUrl) setEditingApiKey(true);
    if (preset.model) setModel(preset.model);
  };

  useEffect(() => {
    if (section !== "provider" || subscription) return;
    let live = true;
    void api
      .subscriptionStatus()
      .then((value) => {
        if (live) setSubscription(value);
      })
      .catch(() => {
        if (live) setSubscription({ signedIn: false, email: "", plan: "", includesCodex: false });
      });
    return () => {
      live = false;
    };
  }, [section, subscription]);

  useEffect(() => {
    if (section !== "account" || account) return;
    let live = true;
    void api
      .accountStatus()
      .then((value) => {
        if (live) setAccount(value);
      })
      .catch(() => {
        if (live) setAccount({ signedIn: false, email: "", origin: "" });
      });
    return () => {
      live = false;
    };
  }, [section, account]);

  useEffect(() => {
    if (section !== "account" || !account?.signedIn || connectors) return;
    let live = true;
    void api
      .accountConnectors()
      .then((value) => {
        if (live) setConnectors(value);
      })
      .catch((error) => {
        if (!live) return;
        if ((error as { kind?: string })?.kind === "signedOut") {
          setAccount({ ...account, signedIn: false, email: "" });
        }
      });
    return () => {
      live = false;
    };
  }, [section, account, connectors]);

  const linkAccount = async () => {
    setStatus(null);
    setLinking(true);
    try {
      const next = await api.signInAccount();
      setAccount(next);
      setConnectors(null);
      setStatus({
        tone: "ok",
        text: next.broker
          ? "Opened the account service."
          : `Signed in as ${next.email || "your Guaca account"}.`,
      });
    } catch (error) {
      setStatus({ tone: "error", text: errorMessage(error) });
    } finally {
      setLinking(false);
      useStore.getState().setHandoff(null);
    }
  };

  const unlinkAccount = async () => {
    setLinking(true);
    setStatus(null);
    try {
      const next = await api.signOutAccount();
      setAccount(next);
      setConnectors(null);
      setStatus({ tone: "ok", text: "Signed out." });
    } catch (error) {
      setStatus({ tone: "error", text: errorMessage(error) });
    } finally {
      setLinking(false);
    }
  };

  const signIn = async () => {
    setStatus(null);
    setBusy(true);
    let code: DeviceCode;
    try {
      code = await api.beginSubscriptionSignin();
      setPendingCode(code);
    } catch (error) {
      setStatus({ tone: "error", text: errorMessage(error) });
      setBusy(false);
      return;
    }

    void openExternal(code.verificationUrl).catch(() => {});

    try {
      const next = await api.completeSubscriptionSignin(code);
      setSubscription(next);
      setPendingCode(null);
      setProvider("chatgpt");
      setStatus({
        tone: next.includesCodex ? "ok" : "error",
        text: next.includesCodex
          ? `Signed in as ${next.email || "your ChatGPT account"}.${
              settings?.provider === "chatgpt" ? "" : " Save to start using it."
            }`
          : `Signed in as ${next.email || "your ChatGPT account"}, but a ${planLabel(next.plan)} plan does not include Codex. Use an API key instead.`,
      });
    } catch (error) {
      setPendingCode(null);
      setStatus({ tone: "error", text: errorMessage(error) });
    } finally {
      setBusy(false);
    }
  };

  const signOut = async () => {
    setBusy(true);
    setStatus(null);
    try {
      const next = await api.signOutSubscription();
      setSettings(next);
      setProvider(next.provider);
      setSubscription({ signedIn: false, email: "", plan: "", includesCodex: false });
      setStatus({ tone: "ok", text: "Signed out." });
    } catch (error) {
      setStatus({ tone: "error", text: errorMessage(error) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="settings" aria-labelledby={titleId} tabIndex={-1} ref={panelRef}>
      <div className="settings__head">
        <h2 className="settings__heading" id={titleId}>
          Settings
        </h2>
        <p className="settings__about">
          Your host runs your agents. Choose where they work, how they connect, and how Guaca
          appears on this device.
        </p>
      </div>

      <label className="mobile-only settings__section field">
        <span className="field__label">Settings section</span>
        <select
          className="input"
          value={section}
          onChange={(event) => setSection(event.target.value as Section)}
        >
          {SECTIONS.map((key) => (
            <option key={key} value={key}>
              {SECTION_LABELS[key]}
            </option>
          ))}
        </select>
      </label>
      <div className="settings__body">
        <div
          className="settings__nav"
          ref={navRef}
          role="tablist"
          aria-orientation="vertical"
          aria-label="Settings sections"
          onKeyDown={moveAlongTabs}
        >
          {SECTIONS.map((key) => (
            <button
              key={key}
              type="button"
              role="tab"
              id={`settings-tab-${key}`}
              className="settings__tab"
              aria-selected={section === key}
              aria-controls="settings-pane"
              tabIndex={section === key ? 0 : -1}
              onClick={() => setSection(key)}
            >
              {SECTION_LABELS[key]}
            </button>
          ))}
        </div>

        <div className="settings__main">
          {notices}
          <div
            className="settings__pane"
            ref={paneRef}
            id="settings-pane"
            role="tabpanel"
            aria-labelledby={`settings-tab-${section}`}
          >
            {section === "general" && (
              <>
                <h3 className="settings__title">General</h3>
                <p className="settings__lede">Who the agents think they are talking to.</p>

                <label className="field">
                  <span
                    className="field__label"
                    title="What every agent calls you, told on every turn. Left blank, they say the operator."
                  >
                    Your name
                  </span>
                  <input
                    className="input"
                    value={operatorName}
                    placeholder="Unnamed"
                    onChange={(event) => setOperatorName(event.target.value)}
                  />
                </label>

                {desktop && (
                  <div className="field">
                    <span className="field__label">Menu bar</span>
                    <p className="field__hint">
                      Guaca keeps an icon there. Its panel shows what is waiting on you and who is
                      working, and takes a reply without bringing this window back.
                    </p>
                  </div>
                )}
              </>
            )}

            {section === "provider" && (
              <>
                <h3 className="settings__title">Provider</h3>
                <p className="settings__lede">
                  Three ways to pay for a turn: a subscription you sign in to, a program already
                  signed in on the host, or an endpoint and a key you paste. What is chosen here is
                  the default. Any group can pay its own way, and one that does is not affected by
                  anything on this page.
                </p>

                <div
                  className="preset preset--plain"
                  aria-current={provider === "chatgpt"}
                  title={
                    subscription?.signedIn
                      ? "Your plan pays for turns, with no per-token bill."
                      : "Sign in and your plan pays for turns, with no per-token bill."
                  }
                >
                  <span className="preset__text">
                    <span className="preset__name">ChatGPT subscription</span>
                    <span className="preset__url">
                      {subscription?.signedIn
                        ? `${subscription.email || "signed in"}${
                            subscription.plan ? ` · ${planLabel(subscription.plan)}` : ""
                          }`
                        : "Your plan pays for turns"}
                    </span>
                  </span>
                  {subscription?.signedIn ? (
                    <span className="preset__actions">
                      {provider === "chatgpt" ? (
                        <span className="preset__state" data-ready="true">
                          In use
                        </span>
                      ) : (
                        <button
                          type="button"
                          className="btn btn--compact"
                          aria-label="Use the ChatGPT subscription"
                          disabled={busy}
                          onClick={() => setProvider("chatgpt")}
                        >
                          Use it
                        </button>
                      )}
                      <button
                        type="button"
                        className="btn btn--compact"
                        disabled={busy}
                        onClick={() => void signOut()}
                      >
                        Sign out
                      </button>
                    </span>
                  ) : (
                    <button
                      type="button"
                      className="btn btn--compact"
                      disabled={busy || pendingCode !== null}
                      onClick={() => void signIn()}
                    >
                      Sign in
                    </button>
                  )}
                </div>

                {pendingCode && (
                  <div className="devicecode" role="status">
                    <p className="devicecode__lede">
                      Enter this code in the browser window that just opened. It expires in fifteen
                      minutes.
                    </p>
                    <p className="devicecode__code">{pendingCode.userCode}</p>
                    <p className="devicecode__url">{pendingCode.verificationUrl}</p>
                    <p className="hint">
                      Only continue if you started this sign-in here. If anything else gave you this
                      code, cancel it.
                    </p>
                  </div>
                )}

                <ClaudeSignin
                  enabled={capabilities.claudeProvider && claudeInstalled === true}
                  row={(action, signedIn) => (
                    <div
                      className="preset preset--plain"
                      aria-current={provider === "claude"}
                      title={
                        !capabilities.claudeProvider
                          ? "Runs the claude program where you signed in, which is your own machine, so a server cannot offer it."
                          : claudeInstalled === false
                            ? "Runs the claude program, which is not installed on this host."
                            : "Runs the claude program, and uses whatever it is signed in to and set to."
                      }
                    >
                      <span className="preset__text">
                        <span className="preset__name">Claude subscription</span>
                        <span className="preset__url">
                          {!capabilities.claudeProvider
                            ? "Only where you signed in"
                            : claudeInstalled === false
                              ? "Install the claude program"
                              : "Runs the claude program"}
                        </span>
                      </span>
                      {!capabilities.claudeProvider ? (
                        <span className="preset__state" data-ready="false">
                          Not on a server
                        </span>
                      ) : claudeInstalled === false ? (
                        <span className="preset__state" data-ready="false">
                          Not installed
                        </span>
                      ) : !signedIn ? (
                        action
                      ) : provider === "claude" ? (
                        <span className="preset__state" data-ready="true">
                          In use
                        </span>
                      ) : (
                        <button
                          type="button"
                          className="btn btn--compact"
                          aria-label="Use the Claude subscription"
                          disabled={busy || claudeInstalled === null}
                          onClick={() => setProvider("claude")}
                        >
                          Use it
                        </button>
                      )}
                    </div>
                  )}
                />

                {provider === "claude" && (
                  <p className="hint">
                    Which model runs, and which account pays, are Claude's own settings. This app
                    passes neither, so a model named here or on an agent is not used while Claude is
                    the provider. Your plan has a rolling quota rather than a per-token bill, and a
                    group reaches it faster than one person does.
                  </p>
                )}

                {subscription?.signedIn && provider === "chatgpt" && (
                  <SubscriptionModel
                    value={subscriptionModel}
                    effort={reasoningEffort}
                    onEffortChange={(effort) => setReasoningEffort(effort ?? "auto")}
                    models={settings?.subscriptionModels ?? []}
                    onChange={setSubscriptionModel}
                    title="The default for any group or agent that does not name one. A subscription has an hourly quota rather than a per-token bill, so a group that talks a lot reaches the ceiling faster than one person would."
                    hint="From your signed-in account."
                  />
                )}

                {provider === "compatible" ? (
                  <>
                    <p className="settings__lede" style={{ marginTop: "1.4rem" }}>
                      Or any OpenAI-compatible endpoint. The ones below are spelled correctly;
                      choosing one fills in the two fields under it, and anything else can be typed
                      in.
                    </p>

                    <ProviderPresets
                      baseUrl={baseUrl}
                      active={true}
                      keySet={Boolean(settings?.apiKeySet)}
                      loopback={capabilities.loopbackEndpoints}
                      onChoose={choose}
                    />

                    <label className="field" style={{ marginTop: "1.1rem" }}>
                      <span className="field__label">Inference endpoint</span>
                      <input
                        className="input"
                        value={baseUrl}
                        placeholder="https://openrouter.ai/api/v1"
                        title="Any OpenAI-compatible base URL. A local server runs without a network."
                        onChange={(event) => {
                          setBaseUrl(event.target.value);
                          setEditingApiKey(true);
                        }}
                      />
                    </label>

                    <ApiKeyField
                      label="API key"
                      title="Stored on the host and never read back into this form. A local server usually wants none. Test connection checks the endpoint and the key."
                      stored={settings?.apiKeySet ?? false}
                      hint={settings?.apiKeyHint ?? ""}
                      value={apiKey}
                      onChange={setApiKey}
                      editing={editingApiKey}
                      onEdit={setEditingApiKey}
                      busy={busy}
                      placeholder="sk-or-v1-…"
                    >
                      {settings?.apiKeySet && editingApiKey
                        ? "Blank keeps the saved key, including when changing endpoints."
                        : undefined}
                    </ApiKeyField>

                    <label className="field">
                      <span className="field__label">Default model</span>
                      <input
                        className="input"
                        value={model}
                        placeholder={providerFor(baseUrl)?.model || "A model your endpoint serves"}
                        title="The default for any group or agent that does not name one."
                        onChange={(event) => setModel(event.target.value)}
                      />
                    </label>
                  </>
                ) : (
                  <div className="preset preset--plain">
                    <span className="preset__text">
                      <span className="preset__name">API provider</span>
                      <span className="preset__url">
                        OpenRouter, local models, or any OpenAI-compatible endpoint
                      </span>
                    </span>
                    <button
                      type="button"
                      className="btn btn--compact"
                      aria-label="Use an API provider"
                      disabled={busy}
                      onClick={() => setProvider("compatible")}
                    >
                      Use it
                    </button>
                  </div>
                )}

                <label className="field">
                  <span className="field__label">Give up on a call after</span>
                  <input
                    className="input"
                    inputMode="numeric"
                    value={timeout}
                    placeholder={`${settings?.requestTimeoutSecs ?? 120} seconds`}
                    title="Seconds to wait for a model call. A slow local model wants more than a hosted one; a run that hangs wants less. Blank keeps what is stored."
                    onChange={(event) => setTimeoutSecs(event.target.value.replace(/[^0-9]/g, ""))}
                  />
                </label>

                <div className="settings__probe">
                  <button type="button" className="btn" disabled={busy} onClick={() => void test()}>
                    Test connection
                  </button>
                  <span className="hint">Tests what is on screen. Save to keep it.</span>
                </div>
              </>
            )}

            {section === "limits" && (
              <>
                <h3 className="settings__title">Limits</h3>
                <p className="settings__lede">
                  Agents that message each other do not stop on their own. These bounds decide when
                  a conversation ends, and every agent is told why when it hits one. A group can set
                  its own; these are what it falls back to.
                </p>

                {LIMITS.map((field) => (
                  <label className="field" key={field.key}>
                    <span className="field__label" title={field.hint}>
                      {field.label}
                    </span>
                    <input
                      className="input input--number"
                      type="number"
                      min={field.min}
                      max={field.max}
                      value={limits[field.key]}
                      onChange={(event) =>
                        setLimits((current) => ({
                          ...current,
                          [field.key]: Number(event.target.value) || field.min,
                        }))
                      }
                    />
                  </label>
                ))}
              </>
            )}

            {section === "machines" && (
              <>
                <h3 className="settings__title">Machines</h3>
                <p className="settings__lede">
                  A sandbox each: a desktop and a terminal an agent can actually work in.
                </p>

                <ApiKeyField
                  label="E2B API key"
                  title="Lets this host rent a computer for an agent: a desktop and a terminal in a sandbox, shown in the corner of its conversation. Which agents have one is set on each agent's Computer panel."
                  stored={settings?.e2bKeySet ?? false}
                  hint={settings?.e2bKeyHint ?? ""}
                  value={e2bKey}
                  onChange={setE2bKey}
                  editing={editingE2bKey}
                  onEdit={setEditingE2bKey}
                  busy={busy}
                  placeholder="e2b_… (optional)"
                >
                  {settings?.e2bKeySet ? undefined : "Without a key, no agent has a computer."}
                </ApiKeyField>

                <label className="field">
                  <span className="field__label">Sleep computers after</span>
                  <input
                    className="input"
                    inputMode="numeric"
                    value={idleMinutes}
                    placeholder={`${settings?.computerIdleMinutes ?? 15} minutes`}
                    title="Idle minutes before a machine sleeps. Sleeping keeps its disk, so a browser stays signed in and wakes where it left off. Only the running time is billed."
                    onChange={(event) => setIdleMinutes(event.target.value.replace(/[^0-9]/g, ""))}
                  />
                </label>

                <ApiKeyField
                  label="Kernel API key"
                  title="Lets this host rent a browser for an agent: a Chrome in the cloud, separate from its computer. It is what agents use for the web, because it tells them where everything on a page is instead of making them aim at pixels. Which agents have one is set on each agent's Browser panel."
                  stored={settings?.kernelKeySet ?? false}
                  hint={settings?.kernelKeyHint ?? ""}
                  value={kernelKey}
                  onChange={setKernelKey}
                  editing={editingKernelKey}
                  onEdit={setEditingKernelKey}
                  busy={busy}
                  placeholder="sk_… (optional)"
                >
                  {settings?.kernelKeySet
                    ? undefined
                    : capabilities.localBrowser
                      ? "Without a key, an agent you give a browser uses one on this Mac: public web only, no sign-ins and no live view."
                      : "Without a key, no agent has a browser."}
                </ApiKeyField>

                <label className="field">
                  <span className="field__label">Close browsers after</span>
                  <input
                    className="input"
                    inputMode="numeric"
                    value={browserIdleMinutes}
                    placeholder={`${settings?.browserIdleMinutes ?? 60} minutes`}
                    title="Idle minutes before a browser is thrown away. It stops billing within seconds of going quiet either way, and what it was signed in to is kept, so the next one opens signed in to the same accounts. Longer only saves the seconds a fresh one takes to start."
                    onChange={(event) =>
                      setBrowserIdleMinutes(event.target.value.replace(/[^0-9]/g, ""))
                    }
                  />
                </label>

                <div
                  className="switch-row"
                  title="For sites that block automation: Kernel disguises the browser and solves captchas. Needs a plan that includes it. Leave it off until a site turns an agent away."
                >
                  <span className="switch-row__text">
                    <span className="switch-row__label">Hide that browsers are automated</span>
                    <span className="switch-row__hint">Costs more per browser.</span>
                  </span>
                  <button
                    type="button"
                    className="toggle"
                    role="switch"
                    aria-label="Hide that browsers are automated"
                    aria-checked={stealth}
                    onClick={() => setStealth(!stealth)}
                  >
                    <span aria-hidden="true" className="toggle__track" />
                  </button>
                </div>
              </>
            )}

            {section === "account" && (
              <>
                <h3 className="settings__title">Account</h3>
                <p className="settings__lede">
                  {account?.broker
                    ? "Your connector runs with your own Google OAuth client and keeps its grants on your host."
                    : `Optional, and it stays that way. Guaca runs on ${hostAt(place)} with your own keys, and everything it does today it does without an account. Signing in adds one thing: guaca.bot holds an OAuth client, so an agent can reach a service that only issues access to a registered application. Gmail is the example. Guaca cannot be that application, because its client secret would be inside a download anybody can read.`}
                </p>

                <div
                  className="preset preset--plain"
                  aria-current={account?.signedIn === true}
                  title="What leaves the host, and only once you have signed in: a request for an access token when an agent uses a connector you authorized. Your conversations, your agents and your keys are not sent anywhere, with or without an account."
                >
                  <span className="preset__text">
                    <span className="preset__name">
                      {account?.broker ? "Your connector" : "Guaca account"}
                    </span>
                    <span className="preset__url">
                      {account?.signedIn
                        ? account.email || (account.broker ? "connected" : "signed in")
                        : account?.broker
                          ? "Connector is not reachable"
                          : "Sign in to connect Gmail, Drive or GitHub"}
                    </span>
                  </span>
                  {account?.signedIn ? (
                    <span className="preset__actions">
                      <button
                        type="button"
                        className="btn btn--compact"
                        disabled={linking}
                        onClick={() =>
                          account.broker
                            ? void linkAccount()
                            : void openExternal(`${account.origin}/app`)
                        }
                      >
                        Manage
                      </button>
                      {!account.broker && (
                        <button
                          type="button"
                          className="btn btn--compact"
                          disabled={linking}
                          title="Forgets the sign-in on this machine immediately."
                          onClick={() => void unlinkAccount()}
                        >
                          Sign out
                        </button>
                      )}
                    </span>
                  ) : (
                    <button
                      type="button"
                      className="btn btn--compact"
                      disabled={linking}
                      onClick={() => void linkAccount()}
                    >
                      {account?.broker ? "Open connector" : "Sign in"}
                    </button>
                  )}
                </div>

                {account?.broker && (
                  <p className="field__hint">
                    To sign out, remove GUACA_BROKER_BEARER from the host environment and restart
                    the host.
                  </p>
                )}

                {linking && (
                  <div className="devicecode" role="status">
                    <p className="devicecode__lede">
                      {account?.broker
                        ? "Finish in the browser window that just opened."
                        : "Finish in the browser window that just opened. Guaca is listening on a port on this machine for the answer, and gives up after five minutes."}
                    </p>
                    <p className="hint">
                      Only continue if you started this here. Nothing else can have opened it.
                    </p>
                  </div>
                )}

                {account?.signedIn && (
                  <>
                    <p className="settings__lede" style={{ marginTop: "1.4rem" }}>
                      What this account has authorized. Ticking and unticking happens at the account
                      service, in a browser, because that is where the consent screens are.
                    </p>
                    {connectors ? (
                      <ul className="settings__list">
                        {connectors.providers.map((provider) => {
                          const granted = provider.capabilities.filter((c) => c.granted);
                          return (
                            <li key={provider.id}>
                              <span className="field__label">{provider.label}</span>
                              <span className="field__hint">
                                {granted.length > 0
                                  ? granted.map((c) => c.label).join(", ")
                                  : "Nothing authorized yet."}
                              </span>
                            </li>
                          );
                        })}
                        {connectors.providers.length === 0 && (
                          <li>
                            <span className="field__hint">
                              This deployment offers no connectors.
                            </span>
                          </li>
                        )}
                      </ul>
                    ) : (
                      <p className="field__hint">Asking the service&hellip;</p>
                    )}
                  </>
                )}

                {account && account.origin !== "https://guaca.bot" && account.origin !== "" && (
                  <p className="field__hint" style={{ marginTop: "1.4rem" }}>
                    {account.broker ? "Your connector is at " : "This build signs in to "}
                    <code>{account.origin}</code>, not guaca.bot.
                  </p>
                )}
              </>
            )}

            {section === "appearance" && (
              <>
                <h3 className="settings__title">Appearance</h3>

                <p className="settings__lede">
                  Applied as you choose, and kept on this machine. None of these is sent anywhere or
                  known to an agent.
                </p>

                <div className="settings-rows">
                  <div
                    className="switch-row"
                    title="The model's published thinking and each tool call, in chat while it works."
                  >
                    <span className="switch-row__text">
                      <span className="switch-row__label">Show reasoning and tool calls</span>
                    </span>
                    <button
                      type="button"
                      className="toggle"
                      role="switch"
                      aria-label="Show reasoning and tool calls"
                      aria-checked={prefs.showReasoning}
                      onClick={() => setPrefs({ showReasoning: !prefs.showReasoning })}
                    >
                      <span aria-hidden="true" className="toggle__track" />
                    </button>
                  </div>
                  <div className="switch-row">
                    <span className="switch-row__text">
                      <span className="switch-row__label">Seasonal looks</span>
                      <span className="switch-row__hint">
                        A holiday palette, greeting and decorations on Thanksgiving and Christmas
                        Day.
                      </span>
                    </span>
                    <button
                      type="button"
                      className="toggle"
                      role="switch"
                      aria-label="Seasonal looks"
                      aria-checked={prefs.seasonal}
                      onClick={() => setPrefs({ seasonal: !prefs.seasonal })}
                    >
                      <span aria-hidden="true" className="toggle__track" />
                    </button>
                  </div>
                  <SettingChoice
                    name="Reading surface"
                    title="The whole window follows this setting."
                    value={prefs.surface}
                    options={(Object.keys(SURFACE_LABELS) as SurfaceMode[]).map((value) => ({
                      value,
                      label: SURFACE_LABELS[value],
                    }))}
                    onChange={(surface) => setLook({ surface })}
                    hint={`${resolveSurface(prefs.surface, systemDark) === "dark" ? "Dark" : "Light"} mode${prefs.surface === "system" ? ", matching your system" : ""}.`}
                  />
                  <SettingChoice
                    name="Theme presets"
                    value={Object.values(THEME_PRESETS).find((preset) =>
                      matchesPreset(prefs, preset),
                    )}
                    options={Object.entries(THEME_PRESETS).map(([label, value]) => ({
                      value,
                      label,
                      name: `Theme: ${label}`,
                    }))}
                    onChange={setLook}
                    hint={"Choose a preset or adjust its colors below."}
                  />
                  <SettingChoice
                    name="Grays"
                    title="The tint in the workspace surfaces and dividers."
                    value={prefs.grays}
                    options={(Object.keys(GRAYS) as GrayKey[]).map((value) => ({
                      value,
                      label: GRAYS[value].label,
                    }))}
                    onChange={(grays) => setLook({ grays })}
                  />
                  <SettingChoice
                    name="Surface temperature"
                    title="Adjust the warmth of the workspace surfaces."
                    value={prefs.temperature}
                    options={TEMPERATURE_KEYS.map((value) => ({
                      value,
                      label: TEMPERATURES[value].label,
                    }))}
                    onChange={(temperature) => setLook({ temperature })}
                  />
                  <SettingChoice
                    name="Attention"
                    title="The one color that means an agent is waiting on you."
                    value={prefs.attention}
                    options={(Object.keys(ATTENTIONS) as AttentionKey[]).map((value) => ({
                      value,
                      label: ATTENTIONS[value].label,
                      preview: (
                        <span
                          className="setting-choice__dot"
                          style={{
                            backgroundColor: themeTokens(
                              { ...prefs, attention: value },
                              resolveSurface(prefs.surface, systemDark),
                            )["--attention-fill"],
                          }}
                        />
                      ),
                    }))}
                    onChange={(attention) => setLook({ attention })}
                  />
                  <SettingChoice
                    name="Contrast"
                    title="More darkens text, hints and borders to the strictest accessibility level, over whichever grays and color you chose."
                    value={prefs.contrast}
                    options={CONTRASTS.map((value) => ({ value, label: CONTRAST_LABELS[value] }))}
                    onChange={(contrast) => setLook({ contrast })}
                  />
                  <SettingChoice
                    name="Typeface"
                    title="System is whatever your computer draws its own windows in. Atkinson Hyperlegible is drawn so that no two letters are easy to mistake for each other."
                    value={prefs.typeface}
                    options={TYPEFACES.map((value) => ({ value, label: TYPEFACE_LABELS[value] }))}
                    onChange={(typeface) => setLook({ typeface })}
                  />
                  <SettingChoice
                    name="Reading face"
                    title="What agents wrote, and what you type to them. Literata is a serif made for reading at length on a screen."
                    value={prefs.reading}
                    options={READINGS.map((value) => ({ value, label: READING_LABELS[value] }))}
                    onChange={(reading) => setLook({ reading })}
                  />
                  <SettingChoice
                    name="Reading size"
                    title="Change the reading text size without resizing the rest of the window."
                    value={prefs.readingSize}
                    options={READING_SIZES.map((value) => ({ value, label: `${value}px` }))}
                    onChange={(readingSize) => setLook({ readingSize })}
                  />
                  <SettingChoice
                    name="Characters"
                    title="Cut uses flat shapes with pupils and lids. Drawn uses a brush line. Each agent keeps the same character."
                    value={prefs.cast}
                    options={CASTS.map((value) => ({
                      value,
                      label: CAST_LABELS[value],
                      preview: (
                        <span className="setting-choice__faces">
                          {CAST_SAMPLE.map((one) => (
                            <AgentAvatar
                              key={one.avatar}
                              avatar={one.avatar}
                              color={one.color}
                              size="xs"
                              mood="idle"
                              cast={value}
                              seed={`settings-${one.avatar}`}
                              plain
                            />
                          ))}
                        </span>
                      ),
                    }))}
                    onChange={(cast) => setPrefs({ cast })}
                  />
                  <SettingChoice
                    name="Interface scale"
                    title="Resize the controls, type and spacing across the window."
                    value={prefs.uiScale}
                    options={UI_SCALES.map((value) => ({ value, label: `${value}%` }))}
                    onChange={(uiScale) => setLook({ uiScale })}
                  />
                </div>
              </>
            )}

            {section === "notifications" && (
              <>
                <h3 className="settings__title">Notifications</h3>
                <p className="settings__lede">
                  The Host decides what reaches you, the same way for every window on it, and one
                  window shows each notification. Nothing interrupts you about the channel you are
                  reading.
                </p>

                {settings && !settings.attention && (
                  <p className="hint">
                    This Host is older than these switches. Update it to choose what notifies you.
                  </p>
                )}

                {settings?.attention &&
                  ATTENTION_ROWS.map((row) => (
                    <div className="switch-row" key={row.key}>
                      <span className="switch-row__text">
                        <span className="switch-row__label">{row.label}</span>
                        <span className="switch-row__hint">{row.hint}</span>
                      </span>
                      <button
                        type="button"
                        className="toggle"
                        role="switch"
                        disabled={attending === row.key}
                        aria-checked={settings.attention?.[row.key]}
                        aria-label={row.label}
                        onClick={() => void toggleAttention(row.key)}
                      >
                        <span aria-hidden="true" className="toggle__track" />
                      </button>
                    </div>
                  ))}

                <div style={{ marginTop: "1rem" }}>
                  {macAccess && (
                    <div style={{ marginBottom: "0.6rem" }}>
                      <p className="hint">{MAC_ACCESS[macAccess]}</p>
                      {macAccess === "denied" && (
                        <button
                          type="button"
                          className="btn btn--compact"
                          style={{ marginTop: "0.4rem" }}
                          onClick={() =>
                            api
                              .openNotificationSettings()
                              .catch((err) => setStatus({ tone: "error", text: errorMessage(err) }))
                          }
                        >
                          Open System Settings
                        </button>
                      )}
                    </div>
                  )}
                  <button
                    type="button"
                    className="btn"
                    onClick={async () => {
                      const { notifyOperator } = await import("../lib/ipc");
                      const sent = await notifyOperator("Guaca", TEST_NOTIFICATION);
                      const access = desktop
                        ? await api.notificationAccess().catch(() => null)
                        : null;
                      if (access !== null) {
                        setMacAccess(access);
                        setStatus(
                          sent
                            ? { tone: "ok", text: "Sent. It should be on your screen now." }
                            : {
                                tone: "error",
                                text:
                                  access === "allowed"
                                    ? NOT_ACCEPTED
                                    : access === "unavailable"
                                      ? MAC_ACCESS.unavailable
                                      : MAC_ACCESS.denied,
                              },
                        );
                        return;
                      }
                      setStatus(
                        sent
                          ? {
                              tone: "info",
                              text: "Handed to the operating system. If nothing appeared, Guaca is not allowed to notify you: check Notifications in System Settings.",
                            }
                          : {
                              tone: "error",
                              text: "This machine refused outright. Allow Guaca to notify you in System Settings, then try again.",
                            },
                      );
                    }}
                  >
                    Send a test notification
                  </button>
                  {!macAccess && (
                    <p className="hint" style={{ marginTop: "0.4rem" }}>
                      The only way to find out whether this machine will show them.
                      {!desktop && " In a browser, this is also where it asks for permission."}
                    </p>
                  )}
                </div>

                {settings?.delivery && (
                  <div style={{ marginTop: "2rem" }}>
                    <h4 className="settings__title">Send to your phone or another service</h4>
                    <p className="settings__lede">
                      While no Guaca window has focus, the Host also sends what would notify you to
                      an address you choose: an ntfy topic, which the free ntfy app shows on your
                      phone, or any service that takes a JSON POST. Nothing is sent while you are at
                      a window.
                    </p>

                    <div className="field">
                      <label className="field__label" htmlFor="push-url">
                        Address
                      </label>
                      {settings.delivery.urlSet && (
                        <div className="api-key__saved">
                          <span className="api-key__status">Address saved</span>
                          <span className="api-key__hint">{settings.delivery.urlHint}</span>
                          <button
                            type="button"
                            className="btn btn--compact"
                            disabled={pushBusy}
                            onClick={async () => {
                              if (await writePush({ url: "", token: "" })) setPushReport(null);
                            }}
                          >
                            Stop sending
                          </button>
                        </div>
                      )}
                      <input
                        id="push-url"
                        className="input"
                        value={pushUrl}
                        placeholder={
                          settings.delivery.urlSet
                            ? "Paste another address to replace it"
                            : "https://ntfy.sh/your-topic"
                        }
                        autoComplete="off"
                        spellCheck={false}
                        disabled={pushBusy}
                        onChange={(event) => setPushUrl(event.target.value)}
                      />
                    </div>

                    <div className="field">
                      <label className="field__label" htmlFor="push-token">
                        Token
                      </label>
                      {settings.delivery.tokenSet && (
                        <div className="api-key__saved">
                          <span className="api-key__status">Token saved</span>
                          <span className="api-key__hint">{settings.delivery.tokenHint}</span>
                          <button
                            type="button"
                            className="btn btn--compact"
                            disabled={pushBusy}
                            onClick={() => void writePush({ token: "" })}
                          >
                            Remove token
                          </button>
                        </div>
                      )}
                      <input
                        id="push-token"
                        className="input"
                        type="password"
                        value={pushToken}
                        placeholder={
                          settings.delivery.tokenSet
                            ? "Enter a replacement token"
                            : "tk_... (optional)"
                        }
                        autoComplete="off"
                        disabled={pushBusy}
                        onChange={(event) => setPushToken(event.target.value)}
                      />
                      <p className="field__hint">
                        Sent as a bearer token. An ntfy access token, or whatever your service asks
                        for.
                      </p>
                    </div>

                    <div className="field">
                      <label className="field__label" htmlFor="push-link">
                        Link
                      </label>
                      <input
                        id="push-link"
                        className="input"
                        value={pushLink ?? settings.delivery.linkBase}
                        placeholder="https://guaca.example.com (optional)"
                        autoComplete="off"
                        spellCheck={false}
                        disabled={pushBusy}
                        onChange={(event) => setPushLink(event.target.value)}
                      />
                      <p className="field__hint">
                        Where tapping a push takes you: this Host's address, if your phone can open
                        it. Leave it empty and a push opens nothing.
                      </p>
                    </div>

                    <div className="settings__probe" style={{ marginTop: 0 }}>
                      <button
                        type="button"
                        className="btn btn--primary"
                        disabled={pushBusy || Object.keys(pushTyped()).length === 0}
                        onClick={() => void savePush()}
                      >
                        Save address
                      </button>
                      <button
                        type="button"
                        className="btn"
                        disabled={pushBusy || !settings.delivery.urlSet}
                        onClick={() => void testPush()}
                      >
                        Send a test
                      </button>
                    </div>
                    {settings.delivery.urlSet &&
                      (() => {
                        const line = pushLine(pushReport, pushTest, Date.now());
                        if (!line) return null;
                        return (
                          <p
                            className="field__hint push-status"
                            role="status"
                            data-tone={line.tone}
                            style={{
                              marginTop: "0.6rem",
                              color: line.tone === "error" ? "var(--danger)" : "var(--muted)",
                            }}
                          >
                            {line.text}
                          </p>
                        );
                      })()}

                    <div className="settings-rows">
                      <SettingChoice
                        name="Format"
                        value={settings.delivery.format}
                        options={PUSH_FORMATS}
                        busy={pushBusy}
                        onChange={(format) => void writePush({ format })}
                      />
                      <SettingChoice
                        name="What a push says"
                        value={settings.delivery.content}
                        options={PUSH_CONTENTS.map((option) => ({ ...option, name: option.label }))}
                        list
                        busy={pushBusy}
                        onChange={(content) => void writePush({ content })}
                        hint="What finished is the agent's own line about it. Choose the second if this address should not carry what your agents are working on."
                      />
                    </div>

                    <p className="field__hint" style={{ marginTop: "1rem" }}>
                      On the public ntfy.sh, anyone who knows a topic can read it. Pick a topic
                      nobody could guess, and better, reserve it and send with an access token, or
                      run your own ntfy server. A server of your own reaches an iPhone instantly
                      only by forwarding a wake-up through ntfy.sh (its upstream-base-url setting),
                      and that wake-up carries a message id, never the text.
                    </p>
                  </div>
                )}
              </>
            )}

            {section === "shortcuts" && (
              <>
                <h3 className="settings__title">Shortcuts</h3>
                <p className="settings__lede">
                  Every key the app answers to. The keys under Anywhere work wherever you are; the
                  rest belong to the surface they are listed under.
                </p>

                <div className="keys">
                  {SURFACES.map((where) => {
                    const rows = BINDINGS.filter((binding) => binding.where === where);
                    if (rows.length === 0) return null;
                    return (
                      <div key={where}>
                        <p className="keys__group">{where}</p>
                        {rows.map((binding) => (
                          <div className="keys__row" key={binding.id}>
                            <span className="keys__what">{binding.what}</span>
                            <span className="keys__combo">{comboLabel(binding)}</span>
                          </div>
                        ))}
                      </div>
                    );
                  })}
                </div>
              </>
            )}

            {visitedPermissions && (
              <div hidden={section !== "permissions"}>
                <Permissions groupId={null} />
              </div>
            )}

            {section === "skills" && (
              <>
                <h3 className="settings__title">Skills</h3>
                <p className="settings__lede">
                  Your instructions for kinds of work, read by every group. A group's own skills are
                  in its settings.
                </p>
                <SkillList scope={{ kind: "workspace" }} />
              </>
            )}

            {section === "workspace" && <WorkspacePane />}
            {section === "compost" && <Compost />}

            {section === "about" && (
              <>
                <div className="about">
                  <h3 className="about__name">Guaca</h3>
                  <p className="about__version">{buildLabel()}</p>
                </div>

                <div className="about__facts">
                  <div className="field">
                    <span className="field__label">What this is</span>
                    <span className="field__hint">
                      A window into a Guaca host. You talk to LLM agents and they talk to each
                      other; <code>guacad</code> runs them, in a container on your own machine or on
                      one you connect to. They keep working while this window is closed, as long as
                      that host is awake.
                    </span>
                  </div>

                  <div className="field">
                    <span className="field__label">Where it keeps things</span>
                    <span className="field__hint">
                      On that host, in its data folder: <code>guac.db</code> for the transcripts,{" "}
                      <code>config.json</code> for the settings and your keys, written so only that
                      host's user account can read it, <code>workspace/</code> for what each agent
                      remembers, and <code>files/</code> for anything attached. {keeps}
                    </span>
                  </div>

                  <div className="field">
                    <span className="field__label">License</span>
                    <span className="field__hint">
                      GNU Affero General Public License v3 or later.
                    </span>
                  </div>
                </div>
              </>
            )}
          </div>

          {status && (
            <div
              className={
                status.tone === "error"
                  ? "banner banner--error"
                  : status.tone === "ok"
                    ? "banner banner--ok"
                    : "banner"
              }
            >
              <span>{status.text}</span>
            </div>
          )}

          {dirty && (
            <div className="settings__foot">
              <button type="button" className="btn" disabled={busy} onClick={discard}>
                Discard changes
              </button>
              <button
                type="button"
                className="btn btn--primary"
                disabled={busy}
                onClick={() => void save()}
              >
                Save
              </button>
            </div>
          )}
        </div>
      </div>
    </section>
  );
}

function WorkspacePane() {
  return (
    <div className="workspace-settings">
      <HostUpdatePanel />
      <HostChoice showUpdate={false} inUse />
      <GroupTransfer />
      <LegacyGroups />
    </div>
  );
}

function ApiKeyField({
  label,
  title,
  stored,
  hint,
  value,
  onChange,
  editing,
  onEdit,
  busy,
  placeholder,
  children,
}: {
  label: string;
  title: string;
  stored: boolean;
  hint: string;
  value: string;
  onChange: (value: string) => void;
  editing: boolean;
  onEdit: (editing: boolean) => void;
  busy: boolean;
  placeholder: string;
  children?: ReactNode;
}) {
  const id = useId();
  const input = useRef<HTMLInputElement>(null);
  const change = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (editing) input.current?.focus();
  }, [editing]);
  return (
    <fieldset className="field api-key">
      <legend className="field__label" title={title}>
        {label}
      </legend>
      {stored && (
        <div className="api-key__saved">
          <span className="api-key__status" role="status">
            API key saved
          </span>
          <span className="api-key__hint">{hint}</span>
          {!editing && (
            <button
              ref={change}
              type="button"
              className="btn btn--compact"
              aria-label={`Change ${label}`}
              disabled={busy}
              onClick={() => onEdit(true)}
            >
              Change key
            </button>
          )}
        </div>
      )}
      {(!stored || editing) && (
        <div className="api-key__entry">
          <input
            ref={input}
            className="input"
            type="password"
            aria-label={label}
            aria-describedby={children ? id : undefined}
            value={value}
            placeholder={stored ? "Enter replacement key" : placeholder}
            autoComplete="off"
            disabled={busy}
            onChange={(event) => onChange(event.target.value)}
          />
          {stored && (
            <button
              type="button"
              className="btn btn--compact"
              aria-label={`Cancel changing ${label}`}
              disabled={busy}
              onClick={() => {
                onChange("");
                onEdit(false);
                requestAnimationFrame(() => change.current?.focus());
              }}
            >
              Cancel
            </button>
          )}
        </div>
      )}
      {children && (
        <p className="field__hint" id={id}>
          {children}
        </p>
      )}
    </fieldset>
  );
}
