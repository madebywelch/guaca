---------------------------- MODULE Relay ----------------------------
EXTENDS Naturals, Sequences, FiniteSets

CONSTANTS
    MaxEnds,
    AckAfterCommit,
    FindFirst,
    CatchUpKeyed,
    PlaceRunOneCommit,
    AmbiguousHeld,
    OrderedByCommit,
    PressChecked,
    Relinks

Said == {"m1"}
Stranger == "s"
Heard == Said \cup {Stranger}
Index(m) == IF m = "m1" THEN 1 ELSE 0
Items == {"reply", "ask", "settle"}
Terminal == {"sent", "failed", "doubt"}
Vias == {"live", "catch"}
People == {"a", "b"}
Range(s) == {s[i] : i \in 1..Len(s)}
Pos(s, x) == CHOOSE i \in 1..Len(s) : s[i] = x

VARIABLES
    up,
    ends,
    acked,
    inflight,
    committed,
    landing,
    catching,
    claimed,
    placeRun,
    high,
    replied,
    made,
    rs,
    fresh,
    awaiting,
    approval,
    told,
    runs,
    shown,
    seq,
    decisions,
    owner,
    askSpot,
    misPressed

memory == <<inflight, committed, landing, catching, fresh, awaiting>>
disk == <<claimed, placeRun, high, replied, made, rs, approval>>
ghosts == <<told, runs, shown, seq, decisions>>
link == <<owner, askSpot, misPressed>>
vars == <<up, ends, acked, memory, disk, ghosts, link>>

Init ==
    /\ up = TRUE /\ ends = 0
    /\ acked = {}
    /\ inflight = {} /\ committed = {} /\ landing = {}
    /\ catching = {}
    /\ fresh = {} /\ awaiting = {}
    /\ claimed = {} /\ placeRun = {} /\ high = 0
    /\ replied = {} /\ made = <<>>
    /\ rs = [i \in Items |-> "none"]
    /\ approval = "none"
    /\ told = {}
    /\ runs = [m \in Said |-> 0]
    /\ shown = [i \in Items |-> 0]
    /\ seq = <<>>
    /\ decisions = 0
    /\ owner = "a" /\ askSpot = "none" /\ misPressed = FALSE

TypeOK ==
    /\ up \in BOOLEAN /\ ends \in 0..MaxEnds
    /\ acked \subseteq Heard
    /\ inflight \subseteq (Heard \X Vias)
    /\ committed \subseteq Heard
    /\ landing \subseteq Said /\ catching \subseteq Heard
    /\ claimed \subseteq Said /\ placeRun \subseteq Said
    /\ high \in 0..1
    /\ replied \subseteq Said
    /\ rs \in [Items -> {"none", "sending", "queued", "sent", "failed", "doubt"}]
    /\ fresh \subseteq Items /\ awaiting \subseteq Items
    /\ approval \in {"none", "pending", "decided"}
    /\ told \subseteq Heard
    /\ owner \in People /\ askSpot \in People \cup {"none"} /\ misPressed \in BOOLEAN

Deliver(m) ==
    /\ up /\ m \notin acked /\ <<m, "live">> \notin inflight
    /\ inflight' = inflight \cup {<<m, "live">>}
    /\ UNCHANGED <<up, ends, acked, committed, landing, catching, fresh, awaiting, disk, ghosts>>
    /\ UNCHANGED link

Catch(m) ==
    /\ up /\ m \in catching
    /\ \A earlier \in catching : Index(earlier) >= Index(m)
    /\ inflight' = inflight \cup {<<m, "catch">>}
    /\ catching' = catching \ {m}
    /\ UNCHANGED <<up, ends, acked, committed, landing, fresh, awaiting, disk, ghosts>>
    /\ UNCHANGED link

Commit(m, via) ==
    /\ up /\ <<m, via>> \in inflight
    /\ inflight' = inflight \ {<<m, via>>}
    /\ committed' = IF via = "live" THEN committed \cup {m} ELSE committed
    /\ IF m = Stranger
       THEN /\ told' = told \cup {m}
            /\ UNCHANGED <<landing, claimed, placeRun, high, runs>>
       ELSE IF m \in claimed /\ (via = "live" \/ CatchUpKeyed)
       THEN UNCHANGED <<told, landing, claimed, placeRun, high, runs>>
       ELSE /\ claimed' = claimed \cup {m}
            /\ runs' = [runs EXCEPT ![m] = @ + 1]
            /\ high' = Index(m)
            /\ IF PlaceRunOneCommit
               THEN placeRun' = placeRun \cup {m} /\ UNCHANGED landing
               ELSE landing' = landing \cup {m} /\ UNCHANGED placeRun
            /\ UNCHANGED told
    /\ UNCHANGED <<up, ends, acked, catching, fresh, awaiting, replied, made, rs, approval,
                   shown, seq, decisions>>
    /\ UNCHANGED link

Land(m) ==
    /\ up /\ m \in landing
    /\ placeRun' = placeRun \cup {m}
    /\ landing' = landing \ {m}
    /\ UNCHANGED <<up, ends, acked, inflight, committed, catching, fresh, awaiting,
                   claimed, high, replied, made, rs, approval, ghosts>>
    /\ UNCHANGED link

Ack(m) ==
    /\ up /\ m \notin acked
    /\ IF AckAfterCommit THEN m \in committed ELSE <<m, "live">> \in inflight \/ m \in committed
    /\ acked' = acked \cup {m}
    /\ committed' = committed \ {m}
    /\ UNCHANGED <<up, ends, inflight, landing, catching, fresh, awaiting, disk, ghosts>>
    /\ UNCHANGED link

Answer(m) ==
    /\ up /\ m = "m1" /\ m \in claimed /\ m \notin replied /\ approval = "decided"
    /\ replied' = replied \cup {m}
    /\ made' = IF m \in placeRun THEN Append(made, "reply") ELSE made
    /\ UNCHANGED <<up, ends, acked, memory, claimed, placeRun, high, rs, approval, ghosts>>
    /\ UNCHANGED link

Asks ==
    /\ up /\ "m1" \in claimed /\ approval = "none"
    /\ approval' = "pending"
    /\ askSpot' = owner
    /\ made' = IF "m1" \in placeRun THEN Append(made, "ask") ELSE made
    /\ UNCHANGED <<up, ends, acked, memory, claimed, placeRun, high, replied, rs, ghosts,
                   owner, misPressed>>

Relink ==
    /\ up /\ Relinks /\ owner = "a"
    /\ owner' = "b"
    /\ UNCHANGED <<up, ends, acked, memory, disk, ghosts, askSpot, misPressed>>

HeadFree(i) ==
    \A j \in Range(made) : Pos(made, j) < Pos(made, i) => rs[j] \in Terminal

Insert(i) ==
    /\ up /\ i \in Range(made) /\ rs[i] = "none" /\ (HeadFree(i) \/ ~OrderedByCommit)
    /\ rs' = [rs EXCEPT ![i] = "sending"]
    /\ fresh' = fresh \cup {i}
    /\ UNCHANGED <<up, ends, acked, inflight, committed, landing, catching, awaiting,
                   claimed, placeRun, high, replied, made, approval, ghosts>>
    /\ UNCHANGED link

Requeue(i) ==
    /\ up /\ rs[i] = "queued"
    /\ rs' = [rs EXCEPT ![i] = "sending"]
    /\ fresh' = fresh \cup {i}
    /\ UNCHANGED <<up, ends, acked, inflight, committed, landing, catching, awaiting,
                   claimed, placeRun, high, replied, made, approval, ghosts>>
    /\ UNCHANGED link

GiveUp(i) ==
    /\ up /\ rs[i] = "queued"
    /\ rs' = [rs EXCEPT ![i] = "failed"]
    /\ UNCHANGED <<up, ends, acked, memory, claimed, placeRun, high, replied, made, approval, ghosts>>
    /\ UNCHANGED link

Post(i) ==
    /\ up /\ i \in fresh /\ i \notin awaiting /\ rs[i] = "sending"
    /\ \/ /\ shown' = [shown EXCEPT ![i] = @ + 1]
          /\ seq' = Append(seq, i)
          /\ awaiting' = awaiting \cup {i}
          /\ UNCHANGED <<rs, fresh>>
       \/ /\ rs' = [rs EXCEPT ![i] = "queued"]
          /\ fresh' = fresh \ {i}
          /\ UNCHANGED <<shown, seq, awaiting>>
       \/ /\ rs' = [rs EXCEPT ![i] = "failed"]
          /\ fresh' = fresh \ {i}
          /\ UNCHANGED <<shown, seq, awaiting>>
       \/ /\ shown' = [shown EXCEPT ![i] = @ + 1]
          /\ seq' = Append(seq, i)
          /\ fresh' = fresh \ {i}
          /\ rs' = IF AmbiguousHeld THEN rs ELSE [rs EXCEPT ![i] = "queued"]
          /\ UNCHANGED awaiting
    /\ UNCHANGED <<up, ends, acked, inflight, committed, landing, catching,
                   claimed, placeRun, high, replied, made, approval, told, runs, decisions>>
    /\ UNCHANGED link

MarkSent(i) ==
    /\ up /\ i \in awaiting
    /\ rs' = [rs EXCEPT ![i] = "sent"]
    /\ awaiting' = awaiting \ {i}
    /\ fresh' = fresh \ {i}
    /\ UNCHANGED <<up, ends, acked, inflight, committed, landing, catching,
                   claimed, placeRun, high, replied, made, approval, ghosts>>
    /\ UNCHANGED link

Recover(i) ==
    /\ up /\ rs[i] = "sending" /\ i \notin fresh
    /\ IF FindFirst
       THEN /\ \/ /\ shown[i] > 0 /\ rs' = [rs EXCEPT ![i] = "sent"]
                  \/ rs' = [rs EXCEPT ![i] = "doubt"]
            /\ UNCHANGED fresh
       ELSE fresh' = fresh \cup {i} /\ UNCHANGED rs
    /\ UNCHANGED <<up, ends, acked, inflight, committed, landing, catching, awaiting,
                   claimed, placeRun, high, replied, made, approval, ghosts>>
    /\ UNCHANGED link

Decide ==
    /\ approval = "pending"
    /\ approval' = "decided"
    /\ decisions' = decisions + 1
    /\ made' = IF "ask" \in Range(made) THEN Append(made, "settle") ELSE made

Rightful(who, where) == who = owner /\ where = askSpot /\ askSpot = owner

Press(who, where) ==
    /\ up /\ shown["ask"] > 0
    /\ PressChecked => Rightful(who, where)
    /\ IF approval = "pending"
       THEN Decide /\ misPressed' = ~Rightful(who, where)
       ELSE UNCHANGED <<approval, decisions, made, misPressed>>
    /\ UNCHANGED <<up, ends, acked, memory, claimed, placeRun, high, replied, rs,
                   told, runs, shown, seq, owner, askSpot>>

AppAnswer ==
    /\ up /\ Decide
    /\ UNCHANGED <<up, ends, acked, memory, claimed, placeRun, high, replied, rs,
                   told, runs, shown, seq>>
    /\ UNCHANGED link

Lapse ==
    /\ up /\ Decide
    /\ UNCHANGED <<up, ends, acked, memory, claimed, placeRun, high, replied, rs,
                   told, runs, shown, seq>>
    /\ UNCHANGED link

End ==
    /\ up /\ ends < MaxEnds
    /\ up' = FALSE /\ ends' = ends + 1
    /\ inflight' = {} /\ committed' = {} /\ landing' = {} /\ catching' = {}
    /\ fresh' = {} /\ awaiting' = {}
    /\ UNCHANGED <<acked, disk, ghosts>>
    /\ UNCHANGED link

Start ==
    /\ ~up
    /\ up' = TRUE
    /\ catching' = Heard
    /\ UNCHANGED <<ends, acked, inflight, committed, landing, fresh, awaiting, disk, ghosts>>
    /\ UNCHANGED link

Next ==
    \/ \E m \in Heard : Deliver(m) \/ Catch(m) \/ Ack(m)
    \/ \E m \in Heard, via \in Vias : Commit(m, via)
    \/ \E m \in Said : Land(m) \/ Answer(m)
    \/ Asks
    \/ \E i \in Items : Insert(i) \/ Requeue(i) \/ GiveUp(i) \/ Post(i)
                        \/ MarkSent(i) \/ Recover(i)
    \/ \E who, where \in People : Press(who, where)
    \/ Relink \/ AppAnswer \/ Lapse
    \/ End \/ Start

Fair ==
    /\ WF_vars(Start)
    /\ WF_vars(Lapse)
    /\ \A i \in Items :
        /\ WF_vars(Insert(i))
        /\ WF_vars(Post(i))
        /\ WF_vars(MarkSent(i))
        /\ WF_vars(Recover(i))
        /\ SF_vars(GiveUp(i))

Spec == Init /\ [][Next]_vars /\ Fair

EventuallyTerminal ==
    \A i \in Items : (i \in Range(made)) ~> (rs[i] \in Terminal)

HeardOnce == \A m \in Said : runs[m] <= 1

HeardOrToldWhy == \A m \in acked : m \in claimed \/ m \in told

ShownOnce == \A i \in Items : shown[i] <= 1

NothingDropped ==
    /\ \A m \in replied : m \in placeRun \/ m \in landing
    /\ \A i \in Items : shown[i] > 0 => rs[i] # "none"
    /\ \A i \in Items : rs[i] = "sending" /\ ~up => i \notin awaiting

InOrder ==
    \A p, q \in 1..Len(seq) :
        p < q => Pos(made, seq[p]) <= Pos(made, seq[q])

DecidedOnce == decisions <= 1

PressedByOwnerAtAsk == ~misPressed

=============================================================================
