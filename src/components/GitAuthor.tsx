import type { GitIdentity } from "../lib/types";

/** Public commit metadata, shared by the clone form and existing repositories. */
export function GitAuthor({
  author,
  disabled,
  onChange,
}: {
  author: GitIdentity;
  disabled: boolean;
  onChange: (author: GitIdentity) => void;
}) {
  return (
    <>
      <label className="field">
        <span className="field__label">Commit author name</span>
        <input
          className="input"
          autoComplete="name"
          value={author.name}
          disabled={disabled}
          onChange={(event) => onChange({ ...author, name: event.target.value })}
        />
      </label>
      <label className="field">
        <span className="field__label">Commit author email</span>
        <input
          className="input"
          type="email"
          autoComplete="email"
          value={author.email}
          disabled={disabled}
          onChange={(event) => onChange({ ...author, email: event.target.value })}
        />
      </label>
      <p className="field__hint">
        Future commits use this identity. Use your GitHub email or GitHub noreply address for
        contribution credit. The address becomes part of commit history. GitHub App pull requests
        are still opened by Guaca.
      </p>
    </>
  );
}
