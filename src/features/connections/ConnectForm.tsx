import { ChevronRight, Info, Lock } from "lucide-react";
import { useRef, useState, type FormEvent } from "react";
import { Button } from "../../components/Button";
import { InlineResult } from "../../components/Feedback";
import { Select, SecretField, TextField } from "../../components/Fields";
import { connectionSave, connectionTest, errorMessage } from "../../ipc/commands";
import type { AppError } from "../../ipc/bindings/AppError";
import type { ConnectionInput } from "../../ipc/bindings/ConnectionInput";
import type { ConnectionSummary } from "../../ipc/bindings/ConnectionSummary";
import type { ErrorCode } from "../../ipc/bindings/ErrorCode";
import type { Jurisdiction } from "../../ipc/bindings/Jurisdiction";
import type { TestResult } from "../../ipc/bindings/TestResult";
import { asAppError } from "../../lib/errors";

type TestState =
  | { kind: "idle" }
  | { kind: "testing" }
  | { kind: "ok"; result: TestResult }
  | { kind: "error"; error: AppError | null; message: string };

const WARNING_CODES: ReadonlySet<ErrorCode> = new Set<ErrorCode>(["accessDenied", "timeout", "rateLimited"]);

const JURISDICTIONS: Array<{ value: Jurisdiction; label: string }> = [
  { value: "default", label: "Default" },
  { value: "eu", label: "EU" },
  { value: "fedramp", label: "FedRAMP" },
];

export interface ConnectFormProps {
  /** Set when editing a saved connection. */
  existing: ConnectionSummary | null;
  /** Start with the secret field open for replacement (edit mode). */
  replaceSecret?: boolean;
  credentialStoreAvailable: boolean;
  onCancel: () => void;
  onSaved: (summary: ConnectionSummary) => void | Promise<void>;
  /** Called when "Where do I find my keys?" is pressed (narrow layouts). */
  onShowHelp?: () => void;
}

interface Errors {
  name?: string;
  accountId?: string;
  accessKeyId?: string;
  secret?: string;
}

/**
 * Connect R2 account form. The secret lives only in this component's state. It is sent
 * with each Test or Save call and cleared after a successful save or when the form closes.
 */
export function ConnectForm({
  existing,
  replaceSecret = false,
  credentialStoreAvailable,
  onCancel,
  onSaved,
  onShowHelp,
}: ConnectFormProps) {
  const editing = existing !== null;
  const [name, setName] = useState(existing?.name ?? "");
  const [accountId, setAccountId] = useState(existing?.accountId ?? "");
  const [accessKeyId, setAccessKeyId] = useState("");
  const [changeAccessKey, setChangeAccessKey] = useState(!editing);
  const [secret, setSecret] = useState("");
  const [secretOpen, setSecretOpen] = useState(!editing || replaceSecret || existing?.secretStorage === "missing");
  const [bucket, setBucket] = useState("");
  const bucketRef = useRef<HTMLInputElement>(null);
  const [jurisdiction, setJurisdiction] = useState<Jurisdiction>(existing?.jurisdiction ?? "default");
  const [advanced, setAdvanced] = useState(existing ? existing.jurisdiction !== "default" : false);
  const [submitted, setSubmitted] = useState(false);
  const [test, setTest] = useState<TestState>({ kind: "idle" });
  const [passed, setPassed] = useState(false);
  const [busy, setBusy] = useState<"test" | "save" | null>(null);

  const touch = <T,>(setter: (v: T) => void) => (v: T) => {
    setter(v);
    setPassed(false);
  };

  const errors: Errors = {};
  const trimmedName = name.trim();
  if (!trimmedName) errors.name = "Enter a name.";
  else if (trimmedName.length > 60) errors.name = "Use 60 characters or fewer.";
  if (!accountId.trim()) errors.accountId = "Enter your Account ID.";
  if (changeAccessKey && !accessKeyId.trim()) errors.accessKeyId = "Enter the Access Key ID.";
  if (secretOpen && !secret) errors.secret = editing ? "Paste the new Secret Access Key." : "Enter the Secret Access Key.";
  const valid = Object.keys(errors).length === 0;
  const unusualAccount = accountId.trim() !== "" && !/^[0-9a-f]{32}$/i.test(accountId.trim());

  const buildInput = (): ConnectionInput => ({
    id: existing?.id ?? null,
    name: trimmedName,
    accountId: accountId.trim(),
    // Empty when editing without changing it: the stored key is kept.
    accessKeyId: changeAccessKey ? accessKeyId.trim() : "",
    // Null when editing without replacing it: the stored secret is kept.
    secret: secretOpen ? secret : null,
    bucket: bucket.trim() || null,
    jurisdiction,
  });

  const fail = (e: unknown) => {
    const appError = asAppError(e);
    setTest({ kind: "error", error: appError, message: errorMessage(e) });
    setPassed(false);
    // Object-level tokens cannot list buckets: guide the user to name them.
    if (appError?.code === "accessDenied" && !bucket.trim()) bucketRef.current?.focus();
  };

  const runTest = async () => {
    setSubmitted(true);
    if (!valid || busy) return;
    setBusy("test");
    setTest({ kind: "testing" });
    // The secret stays only in this form's local state until a save
    // succeeds or the dialog closes, so a failed test can be retried.
    try {
      const result = await connectionTest(buildInput());
      setTest({ kind: "ok", result });
      setPassed(true);
    } catch (e) {
      fail(e);
    } finally {
      setBusy(null);
    }
  };

  const runSave = async () => {
    setSubmitted(true);
    if (!valid || busy) return;
    setBusy("save");
    let saved: ConnectionSummary | null = null;
    try {
      // Saving always runs a connection test in Rust first.
      if (!passed) setTest({ kind: "testing" });
      saved = await connectionSave(buildInput());
    } catch (e) {
      fail(e);
    } finally {
      // Clear the secret once it has been saved.
      if (saved) setSecret("");
      setBusy(null);
    }
    if (saved) await onSaved(saved);
  };

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    void runSave();
  };

  const show = (key: keyof Errors) => (submitted ? (errors[key] ?? null) : null);
  const working = busy !== null;

  const secretHelper = credentialStoreAvailable ? (
    <>
      <Lock size={12} aria-hidden="true" /> Saved in your system keychain
    </>
  ) : (
    <>
      <Lock size={12} aria-hidden="true" /> Kept for this session only
    </>
  );

  return (
    <form className="connect-form" onSubmit={onSubmit} noValidate aria-busy={working}>
      <div className="callout">
        <Info size={16} aria-hidden="true" />
        <p>
          <strong>Use an R2 API token, not your Cloudflare API key or password.</strong> For safety, limit the token to the
          buckets you need.
        </p>
      </div>

      {!credentialStoreAvailable && (
        <InlineResult tone="warning">
          Your system keychain is not available. The secret will be kept in memory until R2Dock closes.
        </InlineResult>
      )}

      <TextField
        label="Name"
        placeholder="Personal"
        value={name}
        onChange={(e) => touch(setName)(e.target.value)}
        error={show("name")}
        maxLength={80}
        autoFocus={!replaceSecret}
        disabled={working}
      />
      <TextField
        label="Account ID"
        info="On your Cloudflare R2 overview page."
        mono
        value={accountId}
        onChange={(e) => touch(setAccountId)(e.target.value)}
        error={show("accountId")}
        helper={unusualAccount ? "Account IDs are usually 32 hexadecimal characters." : undefined}
        disabled={working}
      />

      {changeAccessKey ? (
        <TextField
          label="Access Key ID"
          mono
          value={accessKeyId}
          onChange={(e) => touch(setAccessKeyId)(e.target.value)}
          error={show("accessKeyId")}
          disabled={working}
        />
      ) : (
        <div className="field">
          <span className="field-label">Access Key ID</span>
          <div className="stored-row">
            <span className="mono">{existing?.accessKeyHint}</span>
            <Button size="sm" variant="ghost" onClick={() => setChangeAccessKey(true)} disabled={working}>
              Change
            </Button>
          </div>
        </div>
      )}

      {secretOpen ? (
        <SecretField
          label="Secret Access Key"
          value={secret}
          onChange={touch(setSecret)}
          helper={secretHelper}
          error={show("secret")}
          disabled={working}
          autoFocus={replaceSecret}
        />
      ) : (
        <div className="field">
          <span className="field-label">Secret Access Key</span>
          <div className="stored-row">
            <span className="secondary">
              <Lock size={12} aria-hidden="true" />{" "}
              {existing?.secretStorage === "session" ? "Kept for this session" : "Stored in keychain"}
            </span>
            <Button size="sm" variant="ghost" onClick={() => setSecretOpen(true)} disabled={working}>
              Replace secret
            </Button>
          </div>
        </div>
      )}

      <TextField
        ref={bucketRef}
        label="Buckets (optional)"
        info="Needed for Object Read & Write tokens, which cannot list buckets. Separate several names with commas."
        placeholder="webaudiotv-media, site-assets"
        mono
        value={bucket}
        onChange={(e) => touch(setBucket)(e.target.value)}
        disabled={working}
      />

      <div className="advanced">
        <button type="button" className="disclosure" aria-expanded={advanced} onClick={() => setAdvanced((v) => !v)}>
          <ChevronRight size={14} aria-hidden="true" />
          Advanced
        </button>
        {advanced && (
          <Select
            label="Jurisdiction"
            info="Only change this if your buckets were created in a specific jurisdiction."
            value={jurisdiction}
            onChange={(e) => touch(setJurisdiction)(e.target.value as Jurisdiction)}
            options={JURISDICTIONS}
            disabled={working}
          />
        )}
      </div>

      <div className="connect-result" aria-live="polite">
        {test.kind === "testing" && (
          <InlineResult tone="info">Connecting to R2...</InlineResult>
        )}
        {test.kind === "ok" && (
          <>
            <InlineResult tone="success">{test.result.message}</InlineResult>
            {test.result.warning && <InlineResult tone="warning">{test.result.warning}</InlineResult>}
          </>
        )}
        {test.kind === "error" && (
          <InlineResult
            tone={test.error && WARNING_CODES.has(test.error.code) ? "warning" : "danger"}
            detail={test.error?.detail ?? null}
          >
            {test.message}
          </InlineResult>
        )}
      </div>

      <div className="dialog-footer connect-footer">
        {onShowHelp && (
          <button type="button" className="link-button small connect-help-link" onClick={onShowHelp}>
            Where do I find my keys?
          </button>
        )}
        <Button variant="ghost" onClick={onCancel} disabled={working}>
          Cancel
        </Button>
        <Button variant="secondary" onClick={() => void runTest()} loading={busy === "test"} disabled={busy === "save"}>
          {busy === "test" ? "Testing..." : "Test connection"}
        </Button>
        <Button variant="primary" type="submit" loading={busy === "save"} disabled={busy === "test"}>
          Save connection
        </Button>
      </div>
    </form>
  );
}
