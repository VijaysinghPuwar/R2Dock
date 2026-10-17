import { CircleAlert, Check, Eye, EyeOff, Info, Search, X } from "lucide-react";
import { forwardRef, useId, useState, type InputHTMLAttributes, type ReactNode, type SelectHTMLAttributes } from "react";
import { IconButton } from "./Button";
import { Tooltip } from "./Tooltip";
import "./components.css";

interface FieldShellProps {
  id: string;
  label: ReactNode;
  info?: string;
  helper?: ReactNode;
  error?: string | null;
  children: ReactNode;
  className?: string;
}

function FieldShell({ id, label, info, helper, error, children, className }: FieldShellProps) {
  return (
    <div className={["field", className].filter(Boolean).join(" ")}>
      <div className="field-label-row">
        <label htmlFor={id} className="field-label">
          {label}
        </label>
        {info && (
          <Tooltip content={info} side="top">
            <button type="button" className="info-btn" aria-label={info}>
              <Info size={13} aria-hidden="true" />
            </button>
          </Tooltip>
        )}
      </div>
      {children}
      {error ? (
        <p id={`${id}-error`} className="field-error">
          <CircleAlert size={14} aria-hidden="true" />
          {error}
        </p>
      ) : helper ? (
        <p id={`${id}-helper`} className="field-helper">
          {helper}
        </p>
      ) : null}
    </div>
  );
}

export interface TextFieldProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "id"> {
  label: ReactNode;
  info?: string;
  helper?: ReactNode;
  error?: string | null;
  mono?: boolean;
  success?: boolean;
  fieldClassName?: string;
}

export const TextField = forwardRef<HTMLInputElement, TextFieldProps>(function TextField(
  { label, info, helper, error, mono, success, fieldClassName, className, ...rest },
  ref,
) {
  const id = useId();
  const describedBy = error ? `${id}-error` : helper ? `${id}-helper` : undefined;
  return (
    <FieldShell id={id} label={label} info={info} helper={helper} error={error} className={fieldClassName}>
      <div className="input-wrap">
        <input
          ref={ref}
          id={id}
          className={["input", mono ? "mono" : "", error ? "is-error" : "", className].filter(Boolean).join(" ")}
          aria-invalid={error ? true : undefined}
          aria-describedby={describedBy}
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          {...rest}
        />
        {success && <Check size={14} className="input-success" aria-label="Verified" />}
      </div>
    </FieldShell>
  );
});

export interface SecretFieldProps {
  label: ReactNode;
  value: string;
  onChange: (value: string) => void;
  helper?: ReactNode;
  error?: string | null;
  placeholder?: string;
  disabled?: boolean;
  autoFocus?: boolean;
}

/** Masked secret input with a show/hide toggle. Copy is blocked while masked. */
export const SecretField = forwardRef<HTMLInputElement, SecretFieldProps>(function SecretField(
  { label, value, onChange, helper, error, placeholder, disabled, autoFocus },
  ref,
) {
  const id = useId();
  const [visible, setVisible] = useState(false);
  const describedBy = error ? `${id}-error` : helper ? `${id}-helper` : undefined;
  return (
    <FieldShell id={id} label={label} helper={helper} error={error}>
      <div className="input-wrap has-trailing">
        <input
          ref={ref}
          id={id}
          type={visible ? "text" : "password"}
          className={["input", "mono", error ? "is-error" : ""].filter(Boolean).join(" ")}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          onCopy={(e) => {
            if (!visible) e.preventDefault();
          }}
          onCut={(e) => {
            if (!visible) e.preventDefault();
          }}
          aria-invalid={error ? true : undefined}
          aria-describedby={describedBy}
          autoComplete="new-password"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          placeholder={placeholder}
          disabled={disabled}
          autoFocus={autoFocus}
        />
        <IconButton
          className="input-trailing"
          size="sm"
          label={visible ? "Hide secret" : "Show secret"}
          icon={visible ? <EyeOff size={14} /> : <Eye size={14} />}
          onClick={() => setVisible((v) => !v)}
          disabled={disabled}
        />
      </div>
    </FieldShell>
  );
});

export interface SelectFieldProps extends Omit<SelectHTMLAttributes<HTMLSelectElement>, "id"> {
  label: ReactNode;
  info?: string;
  helper?: ReactNode;
  options: Array<{ value: string; label: string }>;
  fieldClassName?: string;
}

export function Select({ label, info, helper, options, fieldClassName, className, ...rest }: SelectFieldProps) {
  const id = useId();
  return (
    <FieldShell id={id} label={label} info={info} helper={helper} className={fieldClassName}>
      <select id={id} className={["input", "select", className].filter(Boolean).join(" ")} {...rest}>
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </FieldShell>
  );
}

export interface SearchFieldProps {
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
  scope?: { label: string; onToggle: () => void; title: string };
  inputRef?: React.Ref<HTMLInputElement>;
  onKeyDown?: (e: React.KeyboardEvent<HTMLInputElement>) => void;
  label?: string;
}

export function SearchField({ value, onChange, placeholder, scope, inputRef, onKeyDown, label = "Search" }: SearchFieldProps) {
  return (
    <div className="search-field" role="search">
      <Search size={14} className="search-icon" aria-hidden="true" />
      <input
        ref={inputRef}
        type="search"
        className="search-input"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        aria-label={label}
        onKeyDown={onKeyDown}
        autoComplete="off"
        autoCorrect="off"
        autoCapitalize="off"
        spellCheck={false}
      />
      {value && (
        <button type="button" className="search-clear" aria-label="Clear search" onClick={() => onChange("")}>
          <X size={13} aria-hidden="true" />
        </button>
      )}
      {scope && (
        <Tooltip content={scope.title}>
          <button type="button" className="scope-chip" onClick={scope.onToggle} aria-label={`Search scope: ${scope.label}. Change scope`}>
            {scope.label}
          </button>
        </Tooltip>
      )}
    </div>
  );
}
