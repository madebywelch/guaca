---------------------------- MODULE Recovery ----------------------------
EXTENDS Naturals, Sequences, FiniteSets

CONSTANTS
    MaxEnds,
    StartCanFail,
    OperatorStops,
    MarkBeforeRead,
    AcceptOneCommit,
    RecoverUntouched,
    RecoverClears,
    KeepLink,
    SkipStartNote,
    TurnsCanFail

Msgs == <<"m1", "m2">>
MsgSet == {"m1", "m2"}
Run(m) == IF m = "m1" THEN "r1" ELSE "r2"
Runs == {"r1", "r2"}
Earlier(m, n) == m = "m1" /\ n = "m2"
Range(s) == {s[i] : i \in 1..Len(s)}
InOrder(S) == IF S = {"m1", "m2"} THEN <<"m1", "m2">>
              ELSE IF S = {} THEN <<>> ELSE <<CHOOSE m \in S : TRUE>>

NoTurn == [st |-> "none", run |-> "none", batch |-> {}, taken |-> {}, seen |-> {},
           carries |-> FALSE, did |-> FALSE, owed |-> <<>>, fresh |-> {}]

VARIABLES
    up,
    ends,
    draining,
    stopped,
    pdMem,
    outstanding,
    inbox,
    turn,
    resuming,
    accepted,
    pending,
    put,
    whole,
    extra,
    notices,
    read,
    acted,
    answered,
    blind,
    marked,
    calledOff,
    resumedOff,
    riders

host == <<draining, stopped, pdMem, outstanding, inbox, turn, resuming>>
disk == <<accepted, pending, put, whole, extra, notices>>
ghosts == <<read, acted, answered, blind, marked, calledOff, resumedOff>>
vars == <<up, ends, host, disk, ghosts, riders>>

Init ==
    /\ up = TRUE /\ ends = 0
    /\ draining = FALSE /\ stopped = {} /\ pdMem = {}
    /\ outstanding = [r \in Runs |-> 0] /\ inbox = <<>> /\ turn = NoTurn /\ resuming = {}
    /\ accepted = {} /\ pending = [r \in Runs |-> "none"]
    /\ put = [m \in MsgSet |-> "none"] /\ whole = {}
    /\ extra = [r \in Runs |-> 0] /\ notices = [r \in Runs |-> 0]
    /\ read = {} /\ acted = [m \in MsgSet |-> "none"] /\ answered = {} /\ blind = FALSE
    /\ marked = {} /\ calledOff = {} /\ resumedOff = FALSE /\ riders = {}

Halted(r) == draining \/ r \in stopped
MarkStarted(rs) == [r \in Runs |-> IF r \in rs /\ pending[r] = "unstarted"
                                      THEN "started" ELSE pending[r]]
Show(ms) ==
    /\ read' = read \cup ms
    /\ blind' = (blind \/ \E m \in ms :
                    acted[m] /= "none" /\ ~(turn.carries /\ acted[m] = turn.run))

Release(r) ==
    /\ outstanding' = [outstanding EXCEPT ![r] = @ - 1]
    /\ IF outstanding[r] = 1 /\ r \in pdMem
          THEN whole' = whole \cup {r} ELSE whole' = whole
    /\ IF outstanding[r] = 1 /\ r \notin pdMem
          THEN pending' = [pending EXCEPT ![r] = "none"] ELSE pending' = pending

PutDown(batch, taken, records, actedOn) ==
    LET gone == {m \in batch \cup taken : Run(m) \in stopped}
        booked == batch \ gone
        owed == IF booked = {} THEN (actedOn \cap taken) \ gone ELSE {}
        kept == taken \ (gone \cup owed)
        touched == {Run(m) : m \in booked \cup kept}
        left == [r \in Runs |-> outstanding[r] - Cardinality({m \in batch \cup taken : Run(m) = r})]
    IN /\ outstanding' = left
       /\ pdMem' = pdMem \cup touched
       /\ put' = [m \in MsgSet |-> IF m \in booked THEN (IF records THEN "rec" ELSE "bare")
                                   ELSE IF m \in kept THEN (IF KeepLink /\ m \in actedOn /\ records THEN "link" ELSE "bare")
                                   ELSE put[m]]
       /\ whole' = whole \cup {r \in touched : left[r] = 0}
       /\ pending' = [r \in Runs |-> IF r \in {Run(m) : m \in gone \cup owed} /\ left[r] = 0
                                      THEN "none" ELSE pending[r]]
       /\ notices' = [r \in Runs |-> IF r \in {Run(m) : m \in owed} THEN notices[r] + 1 ELSE notices[r]]

---------------------------------------------------------------------------
Send ==
    /\ riders' = riders
    /\ up
    /\ \E m \in MsgSet \ accepted :
        /\ \A n \in MsgSet : Earlier(n, m) => n \in accepted
        /\ accepted' = accepted \cup {m}
        /\ IF draining
              THEN /\ pending' = [pending EXCEPT ![Run(m)] = "unstarted"]
                   /\ put' = [put EXCEPT ![m] = "bare"]
                   /\ whole' = whole \cup {Run(m)}
                   /\ pdMem' = pdMem \cup {Run(m)}
                   /\ UNCHANGED <<outstanding, inbox>>
              ELSE /\ pending' = IF AcceptOneCommit
                                    THEN [pending EXCEPT ![Run(m)] = "unstarted"]
                                    ELSE pending
                   /\ outstanding' = [outstanding EXCEPT ![Run(m)] = @ + 1]
                   /\ inbox' = Append(inbox, m)
                   /\ UNCHANGED <<put, whole, pdMem>>
    /\ UNCHANGED <<up, ends, draining, stopped, turn, resuming, extra, notices, ghosts>>

AcceptLate ==
    /\ riders' = riders
    /\ up /\ ~AcceptOneCommit
    /\ \E m \in accepted :
        /\ pending[Run(m)] = "none" /\ outstanding[Run(m)] > 0
        /\ pending' = [pending EXCEPT ![Run(m)] = "unstarted"]
    /\ UNCHANGED <<up, ends, host, accepted, put, whole, extra, notices, ghosts>>

Dequeue ==
    /\ riders' = riders
    /\ up /\ turn.st = "none" /\ inbox /= <<>> /\ ~draining
    /\ turn' = [NoTurn EXCEPT !.st = "dequeued", !.run = Run(Head(inbox)),
                              !.batch = {Head(inbox)}]
    /\ inbox' = Tail(inbox)
    /\ UNCHANGED <<up, ends, draining, stopped, pdMem, outstanding, resuming, disk, ghosts>>

PutDownQueued ==
    /\ up /\ turn.st = "none" /\ inbox /= <<>> /\ draining
    /\ LET m == Head(inbox) IN
       /\ inbox' = Tail(inbox)
       /\ resuming' = resuming \ {m}
       /\ LET rides == IF m \in resuming THEN riders ELSE {} IN
          /\ riders' = riders \ rides
          /\ PutDown({m}, rides, m \in resuming, rides)
          /\ extra' = IF Run(m) \in stopped THEN [extra EXCEPT ![Run(m)] = @ + 1] ELSE extra
    /\ UNCHANGED <<up, ends, draining, stopped, turn, accepted, ghosts>>

MarkBatch ==
    /\ riders' = riders
    /\ up /\ turn.st = "dequeued"
    /\ pending' = IF MarkBeforeRead THEN MarkStarted({turn.run}) ELSE pending
    /\ marked' = IF MarkBeforeRead THEN marked \cup {turn.run} ELSE marked
    /\ turn' = [turn EXCEPT !.st = "marked"]
    /\ UNCHANGED <<up, ends, draining, stopped, pdMem, outstanding, inbox, resuming,
                   accepted, put, whole, extra, notices,
                   read, acted, answered, blind, calledOff, resumedOff>>

MarkFails ==
    /\ up /\ StartCanFail /\ turn.st = "dequeued"
    /\ resuming' = resuming \ turn.batch
    /\ LET rides == IF turn.batch \cap resuming /= {} THEN riders ELSE {} IN
          /\ riders' = riders \ rides
          /\ PutDown(turn.batch, rides, turn.batch \cap resuming /= {}, rides)
    /\ extra' = IF SkipStartNote \/ turn.run \in stopped THEN extra
                ELSE [extra EXCEPT ![turn.run] = @ + 1]
    /\ turn' = NoTurn
    /\ UNCHANGED <<up, ends, draining, stopped, inbox, accepted, ghosts>>

Begin ==
    /\ up /\ turn.st = "marked"
    /\ resuming' = resuming \ turn.batch
    /\ LET carries == turn.batch \cap resuming /= {}
           rides == IF carries THEN riders ELSE {}
       IN /\ riders' = riders \ rides
          /\ IF Halted(turn.run)
              THEN /\ turn' = NoTurn
                   /\ PutDown(turn.batch, rides, carries, rides)
                   /\ extra' = IF turn.run \in stopped THEN [extra EXCEPT ![turn.run] = @ + 1] ELSE extra
              ELSE /\ turn' = [turn EXCEPT !.st = "round", !.carries = carries, !.taken = rides]
                   /\ UNCHANGED <<outstanding, pdMem, put, whole, pending, extra, notices>>
    /\ UNCHANGED <<up, ends, draining, stopped, inbox, accepted, ghosts>>

Takeable == inbox /= <<>> /\ ~Halted(Run(Head(inbox))) /\ Head(inbox) \notin resuming

TakeIn ==
    /\ riders' = riders
    /\ up /\ turn.st = "round" /\ ~Halted(turn.run) /\ Takeable
    /\ LET m == Head(inbox) IN
       /\ pending' = MarkStarted({Run(m)})
       /\ marked' = marked \cup {Run(m)}
       /\ inbox' = Tail(inbox)
       /\ turn' = [turn EXCEPT !.taken = @ \cup {m}, !.fresh = @ \cup {m}]
    /\ UNCHANGED <<up, ends, draining, stopped, pdMem, outstanding, resuming,
                   accepted, put, whole, extra, notices,
                   read, acted, answered, blind, calledOff, resumedOff>>

Think ==
    /\ riders' = riders
    /\ up /\ turn.st = "round" /\ ~Halted(turn.run) /\ ~Takeable
    /\ Show((turn.batch \cup turn.taken) \ turn.seen)
    /\ turn' = [turn EXCEPT !.st = "thinking", !.seen = turn.batch \cup turn.taken]
    /\ UNCHANGED <<up, ends, draining, stopped, pdMem, outstanding, inbox, resuming,
                   disk, acted, answered, marked, calledOff, resumedOff>>

Call ==
    /\ riders' = riders
    /\ up /\ turn.st = "thinking"
    /\ acted' = [m \in MsgSet |-> IF m \in turn.seen THEN turn.run ELSE acted[m]]
    /\ turn' = [turn EXCEPT !.st = "acting", !.did = TRUE, !.fresh = {}]
    /\ UNCHANGED <<up, ends, draining, stopped, pdMem, outstanding, inbox, resuming,
                   disk, read, answered, blind, marked, calledOff, resumedOff>>

Returned ==
    /\ riders' = riders
    /\ up /\ turn.st = "acting"
    /\ turn' = [turn EXCEPT !.st = "round"]
    /\ pending' = IF MarkBeforeRead THEN pending ELSE MarkStarted({turn.run})
    /\ marked' = IF MarkBeforeRead THEN marked ELSE marked \cup {turn.run}
    /\ UNCHANGED <<up, ends, draining, stopped, pdMem, outstanding, inbox, resuming,
                   accepted, put, whole, extra, notices,
                   read, acted, answered, blind, calledOff, resumedOff>>

CalledOff ==
    /\ riders' = riders
    /\ up /\ turn.st \in {"round", "thinking"} /\ Halted(turn.run)
    /\ turn' = [turn EXCEPT !.st = "halted"]
    /\ UNCHANGED <<up, ends, draining, stopped, pdMem, outstanding, inbox, resuming,
                   disk, ghosts>>

Answer ==
    /\ riders' = riders
    /\ up /\ turn.st = "thinking"
    /\ extra' = [extra EXCEPT ![turn.run] = @ + 1]
    /\ answered' = answered \cup turn.seen
    /\ turn' = [turn EXCEPT !.st = "finishing",
                            !.owed = IF turn.taken = {} THEN <<turn.run>>
                                     ELSE <<Run(CHOOSE m \in turn.taken : TRUE), turn.run>>]
    /\ UNCHANGED <<up, ends, draining, stopped, pdMem, outstanding, inbox, resuming,
                   accepted, pending, put, whole, notices,
                   read, acted, blind, marked, calledOff, resumedOff>>

HaltStopped ==
    /\ riders' = riders
    /\ up /\ turn.st = "halted" /\ turn.run \in stopped
    /\ extra' = [extra EXCEPT ![turn.run] = @ + 1]
    /\ LET ended == {m \in turn.taken : Run(m) \in stopped}
           unread == (turn.taken \ ended) \cap turn.fresh
           owed == turn.taken \ (ended \cup unread)
       IN /\ inbox' = InOrder(unread) \o inbox
          /\ LET left == [r \in Runs |-> outstanding[r] - Cardinality({m \in owed : Run(m) = r})]
             IN /\ outstanding' = left
                /\ pending' = [r \in Runs |-> IF r \in {Run(m) : m \in owed} /\ left[r] = 0
                                                 THEN "none" ELSE pending[r]]
          /\ notices' = [r \in Runs |-> IF r \in {Run(m) : m \in owed} THEN notices[r] + 1 ELSE notices[r]]
          /\ turn' = [turn EXCEPT !.st = "finishing",
                                  !.owed = IF ended = {} THEN <<turn.run>>
                                           ELSE <<Run(CHOOSE m \in ended : TRUE), turn.run>>]
    /\ UNCHANGED <<up, ends, draining, stopped, pdMem, resuming,
                   accepted, put, whole, ghosts>>

TurnEnds ==
    /\ riders' = riders
    /\ up /\ TurnsCanFail /\ turn.st \in {"round", "thinking"} /\ ~Halted(turn.run)
    /\ extra' = [extra EXCEPT ![turn.run] = @ + 1]
    /\ answered' = answered \cup turn.batch
    /\ LET ended == {m \in turn.taken : Run(m) \in stopped}
           unread == (turn.taken \ ended) \cap turn.fresh
           owed == turn.taken \ (ended \cup unread)
           left == [r \in Runs |-> outstanding[r] - Cardinality({m \in owed : Run(m) = r})]
       IN /\ inbox' = InOrder(unread) \o inbox
          /\ outstanding' = left
          /\ pending' = [r \in Runs |-> IF r \in {Run(m) : m \in owed} /\ left[r] = 0
                                           THEN "none" ELSE pending[r]]
          /\ notices' = [r \in Runs |-> IF r \in {Run(m) : m \in owed} THEN notices[r] + 1 ELSE notices[r]]
          /\ turn' = [turn EXCEPT !.st = "finishing",
                                  !.owed = IF ended = {} THEN <<turn.run>>
                                           ELSE <<Run(CHOOSE m \in ended : TRUE), turn.run>>]
    /\ UNCHANGED <<up, ends, draining, stopped, pdMem, resuming,
                   accepted, put, whole, read, acted, blind, marked, calledOff, resumedOff>>

FinishOne ==
    /\ riders' = riders
    /\ up /\ turn.st = "finishing"
    /\ Release(Head(turn.owed))
    /\ turn' = IF Len(turn.owed) = 1 THEN NoTurn ELSE [turn EXCEPT !.owed = Tail(@)]
    /\ UNCHANGED <<up, ends, draining, stopped, pdMem, inbox, resuming,
                   accepted, put, extra, notices, ghosts>>

PutTurnDown ==
    /\ riders' = riders
    /\ up /\ turn.st = "halted" /\ turn.run \notin stopped
    /\ extra' = [extra EXCEPT ![turn.run] = @ + 1]
    /\ turn' = [turn EXCEPT !.st = "recorded"]
    /\ UNCHANGED <<up, ends, draining, stopped, pdMem, outstanding, inbox, resuming,
                   accepted, pending, put, whole, notices, ghosts>>

PutTurnDownCommit ==
    /\ riders' = riders
    /\ up /\ turn.st = "recorded"
    /\ PutDown(turn.batch, turn.taken, turn.did \/ turn.carries, turn.taken \ turn.fresh)
    /\ turn' = NoTurn
    /\ UNCHANGED <<up, ends, draining, stopped, inbox, resuming,
                   accepted, extra, ghosts>>

Stop ==
    /\ riders' = riders
    /\ up /\ OperatorStops
    /\ \E r \in Runs :
        /\ outstanding[r] > 0 /\ r \notin stopped
        /\ stopped' = stopped \cup {r}
        /\ calledOff' = calledOff \cup {r}
        /\ pending' = [pending EXCEPT ![r] = "none"]
        /\ put' = [m \in MsgSet |-> IF Run(m) = r THEN "none" ELSE put[m]]
        /\ whole' = whole \ {r}
        /\ pdMem' = pdMem \ {r}
    /\ UNCHANGED <<up, ends, draining, outstanding, inbox, turn, resuming,
                   accepted, extra, notices, read, acted, answered, blind, marked, resumedOff>>

Drain ==
    /\ riders' = riders
    /\ up /\ ~draining
    /\ draining' = TRUE
    /\ UNCHANGED <<up, ends, stopped, pdMem, outstanding, inbox, turn, resuming, disk, ghosts>>

End ==
    /\ up /\ ends < MaxEnds
    /\ up' = FALSE /\ ends' = ends + 1
    /\ draining' = FALSE /\ stopped' = {} /\ pdMem' = {}
    /\ outstanding' = [r \in Runs |-> 0] /\ inbox' = <<>> /\ turn' = NoTurn
    /\ resuming' = {} /\ riders' = {}
    /\ UNCHANGED <<disk, ghosts>>

Recover ==
    /\ ~up
    /\ LET putM == {m \in MsgSet : put[m] /= "none"}
           linked == {m \in putM : put[m] = "link"}
           carrierOk == \E c \in MsgSet : put[c] = "rec" /\ Run(c) \in whole
           ride == IF carrierOk THEN {m \in linked : Run(m) \in whole} ELSE {}
           carried == {m \in putM : Run(m) \in whole} \ linked
           cut == putM \ (carried \cup ride)
           untouched == IF RecoverUntouched
                           THEN {m \in accepted : pending[Run(m)] = "unstarted"
                                   /\ extra[Run(m)] = 0 /\ m \notin carried}
                           ELSE {}
           resumed == carried \cup untouched
           picked == {Run(m) : m \in resumed \cup ride}
           withRec == {m \in carried : put[m] = "rec"}
           anchors == ({Run(m) : m \in {n \in accepted : pending[Run(n)] /= "none"}}
                        \cup {Run(m) : m \in cut}) \ picked
           p1 == MarkStarted({Run(m) : m \in withRec})
       IN /\ pending' = [r \in Runs |->
                IF r \in picked THEN (IF p1[r] = "none" THEN "unstarted" ELSE p1[r])
                ELSE IF RecoverClears THEN "none" ELSE p1[r]]
          /\ notices' = [r \in Runs |-> IF r \in anchors THEN notices[r] + 1 ELSE notices[r]]
          /\ extra' = [r \in Runs |-> IF r \in anchors THEN extra[r] + 1 ELSE extra[r]]
          /\ put' = [m \in MsgSet |-> "none"]
          /\ whole' = {}
          /\ inbox' = InOrder(resumed)
          /\ outstanding' = [r \in Runs |-> Cardinality({m \in resumed \cup ride : Run(m) = r})]
          /\ riders' = ride
          /\ resuming' = withRec
          /\ marked' = marked \cup {Run(m) : m \in withRec}
          /\ resumedOff' = (resumedOff \/ picked \cap calledOff /= {})
    /\ up' = TRUE
    /\ UNCHANGED <<ends, draining, stopped, pdMem, turn, accepted,
                   read, acted, answered, blind, calledOff>>

Next ==
    \/ Send \/ AcceptLate \/ Dequeue \/ PutDownQueued \/ MarkBatch \/ MarkFails
    \/ Begin \/ TakeIn \/ Think \/ Call \/ Returned \/ CalledOff \/ Answer
    \/ HaltStopped \/ TurnEnds \/ FinishOne \/ PutTurnDown \/ PutTurnDownCommit
    \/ Stop \/ Drain \/ End \/ Recover

Spec == Init /\ [][Next]_vars

---------------------------------------------------------------------------
NoBlindRepeat == ~blind
NothingLost ==
    \A m \in accepted :
        \/ Run(m) \in calledOff \/ m \in answered
        \/ m \in Range(inbox) \/ m \in turn.batch \/ m \in turn.taken
        \/ pending[Run(m)] /= "none" \/ put[m] /= "none" \/ notices[Run(m)] > 0
OnlyStartedReported == \A r \in Runs : notices[r] > 0 => r \in marked
ReportedOnce == \A r \in Runs : notices[r] <= 1
StoppedStaysStopped == ~resumedOff

TypeOK ==
    /\ pending \in [Runs -> {"none", "unstarted", "started"}]
    /\ put \in [MsgSet -> {"none", "bare", "rec", "link"}]
    /\ \A r \in Runs : outstanding[r] >= 0
=============================================================================
