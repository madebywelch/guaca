# The workspace

What the operator sees, and the decisions the webview makes on its own. The
runtime half is in `ARCHITECTURE.md`; `src/lib/transcript.ts` is the file to
read first.

## A channel names nobody, and that is not a missing feature

It has two participants: the agent it is named after, at the top of the pane,
and the person reading it. A name and a clock over every message is two lines of
chrome carrying one fact, and four replies written inside the same minute drew
four of them. The portrait says which agent and the side of the column says
whether the words are yours. `named` on `MessageItem` is where the two views
part: a channel passes `false`, and the pair's own thread takes the default,
because there both participants are agents and neither is the reader. The clock
went with them: it is a hover on the row, and `transcriptRows` draws one line
where the silence ran past half an hour, which is the only place a time ever
changed what the operator understood. That line also ends whatever burst was
open, because two exchanges three hours apart are two things that happened.

What a channel folds and what it must never fold is in *A channel says an
exchange happened; the pair's thread is what it said*, in `ARCHITECTURE.md`.

## Pinning is where a row is drawn and nothing else

It does not bump the card version, because the version is how a peer notices a
card changed under it and nothing a peer can read has. A pinned agent is lifted
out of its group in the rail and still counted in it, because it is still in it:
same wall, same bill, same peers. Two rows for one agent would be two nodes in
the sidebar's `rowRefs`, and the wire would have to pick one to throw a message
at.

## A duplicate copies the card and nothing an agent went and did

Look, model, skills and instructions; not the sandbox, the memory, the schedule,
the accounts or the transcript. Two agents holding one sandbox id is two agents
on one machine, and a copy that inherited a routine would double a standing
commitment nobody asked to double.

## Search happens in two places and is ranked in one

The workspace is held in two places, so it is matched in two: messages, files,
links and routines are in SQLite and are matched there, while agents and groups
are already in the webview's store to draw the rail and actions are not stored
anywhere at all. Reading the transcript into the renderer to search it would
copy the database across IPC on every keystroke; going to IPC for two agent
names would make the commonest search the slow one. What must not be split is
the ordering: both halves arrive in `lib/search.ts` as raw matches and are
scored by one function, because a list where an agent and a message are ordered
by different rules is a list you have to read twice. A file and a link are the
same rows as the messages read from a different angle, which is why one scan
produces all three.

## A search hit that opens the wrong part of a channel is a search that failed

A transcript is read as "the newest three hundred", and a hit from last month is
not in that window. `channel_messages` takes a `through` so the window reaches
back to the message being opened, bounded at a thousand; past that the operator
lands in the right channel at its newest end. Anything that jumps to a message
goes through `openMessage` rather than `select`.
