# Attachments

Files are references: what a model gets depends on what they are, and a reply
that carries one has to read back as having carried it. `src-tauri/src/files.rs`,
`Runtime::emit_reply` and `src/components/FileCard.tsx`. The reading view's own
CSS is in `styles.md`, because that is where it fails.

- **`FileCard` hands the frame a `blob:` URL on purpose.** WebKit refuses a
  custom scheme in a frame and says nothing. A direct `guacfile:` `src` passes
  every test in this repo and draws an empty rectangle.
- **Every `guacfile:` answer carries `access-control-allow-origin`, refusals
  included.** A custom scheme is cross-origin to the page that asked, so without
  it a `fetch` rejects with `TypeError: Load failed` and never sees the status.
  An `img` is exempt and is the only preview that is not a `fetch`, so dropping
  the header shows as pictures drawing and every document, log and PDF failing,
  each with the one error message that cannot say which failure it was.
- **`emit_reply` delivers a reply that carries a file and no text.** Handing over
  a document with nothing typed is normal, and judging the reply empty by its
  text alone drops the thing the turn was spent producing.
- **`body_with_files` names a file on an agent's own turns too, not just on
  incoming ones.** An agent that reads its last turn back without the file it
  attached has no record of handing anything over, so it attaches the document
  again and reports it as the first time.
