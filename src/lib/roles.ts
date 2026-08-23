/**
 * What an agent is for, in the only vocabulary OpenRouter ranks models in.
 *
 * OpenRouter classifies the traffic it routes into twelve use cases and will
 * hand back the models doing each of them. An agent, meanwhile, is a name, a
 * list of skills and a page of instructions. This is the join: it reads what
 * the operator has already written and answers with one of the twelve, or with
 * nothing.
 *
 * ## Nothing is an answer, and the common one
 *
 * Most agents are not any of the twelve. A Manager, a Router, an Inbox: they do
 * work OpenRouter has no category for, and the honest reply is silence. A
 * scoring function that always names its best guess would put a legal model
 * under a scheduling agent and give the operator a reason to distrust every
 * other suggestion it makes. So there is a floor, and a tie is not a winner.
 *
 * ## Why this is not a model call
 *
 * Deriving a role by asking a model would cost money and a round trip every
 * time a dialog opens, to answer a question a keyword can answer, and would
 * answer it differently on two openings of the same dialog. It is also the one
 * place a wrong answer is invisible: an operator cannot tell a considered
 * "marketing" from a guessed one. Keywords are free, instant, the same twice,
 * and wrong in ways a person can see and ignore.
 *
 * ## Sales
 *
 * There is no sales category. OpenRouter's twelve go marketing, marketing/seo
 * and no further, so sales vocabulary is scored into marketing: the models
 * people send outreach and pitch copy to are the models they send campaign copy
 * to. Leaving it unmatched would mean an agent called Sales, which is the second
 * thing anybody builds here, gets nothing and reads as broken.
 */

/** One of the twelve, and how it is said in a sentence about an agent. */
export interface Role {
  /** The category slug, spelled as OpenRouter spells it. Crosses IPC. */
  id: string;
  /** Fits "reads as ___ work", which is the only sentence it appears in. */
  label: string;
}

/**
 * The twelve, in OpenRouter's own spelling.
 *
 * `llm/catalog.rs` holds the same list because it refuses anything else
 * before spending a request, and `ipc.contract.test.ts` compares the two files.
 * Neither is the source: OpenRouter is, and the pair failing together is how a
 * use case renamed there is noticed here.
 */
export const ROLES: Role[] = [
  { id: "programming", label: "programming" },
  { id: "roleplay", label: "roleplay" },
  { id: "marketing", label: "marketing" },
  { id: "marketing/seo", label: "SEO" },
  { id: "technology", label: "technology" },
  { id: "science", label: "science" },
  { id: "translation", label: "translation" },
  { id: "legal", label: "legal" },
  { id: "finance", label: "finance" },
  { id: "health", label: "health" },
  { id: "trivia", label: "trivia" },
  { id: "academia", label: "academic" },
];

/**
 * What the dialog knows about an agent while it is being written.
 *
 * The draft rather than the saved card, so a suggestion follows what is being
 * typed. An operator naming a new agent "Counsel" should see the answer before
 * they press save, not after.
 */
export interface Evidence {
  name: string;
  skills: string[];
  instructions: string;
}

/**
 * How much each field is worth per distinct word it matches.
 *
 * A name and a skill list are short and deliberate: every word in them was
 * chosen to describe the agent, so one hit in either is enough on its own.
 * Instructions are prose about the work, and prose mentions things in passing,
 * so two separate words are wanted before it decides anything.
 */
const WEIGHT = { name: 5, skills: 4, instructions: 2 } as const;

/** One name hit, or one skill hit, or two words of prose. */
const FLOOR = 4;

/**
 * The words that mean each use case.
 *
 * Nouns of the trade rather than anything an agent might mention once. Words
 * that are ordinary English on their own are spelled as the phrase they belong
 * to: "lead" is a verb every manager's instructions use and "lead generation"
 * is not, so only the second is here.
 *
 * Overlap between lists is fine and expected. "devops" is both programming and
 * technology, and an agent with one word of evidence either way should get no
 * suggestion rather than an arbitrary one: that is the tie rule below doing its
 * job, not a table that needs tidying.
 *
 * These are matched against what an operator typed, so both spellings of a word
 * that has two are here. That is the one list in this repo the American-spelling
 * convention does not reach: an operator who writes "localisation" is describing
 * the same agent as one who writes "localization", and dropping the first is a
 * suggestion silently not made.
 */
const WORDS: Record<string, string[]> = {
  programming: [
    "code",
    "coding",
    "coder",
    "codebase",
    "developer",
    "dev",
    "engineer",
    "engineering",
    "software",
    "programmer",
    "programming",
    "debug",
    "debugging",
    "refactor",
    "refactoring",
    "api",
    "backend",
    "frontend",
    "typescript",
    "python",
    "rust",
    "golang",
    "sql",
    "repo",
    "repository",
    "pull request",
    "code review",
    "devops",
    "compiler",
    "unit test",
    "unit tests",
    "git",
  ],
  roleplay: [
    "roleplay",
    "role play",
    "in character",
    "persona",
    "storytelling",
    "fiction",
    "narrative",
    "dungeon master",
    "npc",
    "improv",
    "companion",
    "worldbuilding",
    "screenplay",
    "dialogue writing",
  ],
  marketing: [
    "marketing",
    "marketer",
    "brand",
    "branding",
    "campaign",
    "campaigns",
    "copywriting",
    "copywriter",
    "copy",
    "advertising",
    "advert",
    "adverts",
    "social media",
    "newsletter",
    "positioning",
    "messaging",
    "growth",
    // Sales has no category of its own. See the module comment.
    "sales",
    "salesperson",
    "selling",
    "prospect",
    "prospecting",
    "outreach",
    "outbound",
    "cold email",
    "crm",
    "lead generation",
    "pitch deck",
    "go to market",
    "customer acquisition",
  ],
  "marketing/seo": [
    "seo",
    "search engine",
    "keyword research",
    "keywords",
    "serp",
    "backlink",
    "backlinks",
    "meta description",
    "organic traffic",
    "search ranking",
    "content marketing",
    "link building",
    "on page",
    "site audit",
  ],
  technology: [
    "technology",
    "tech",
    "infrastructure",
    "sysadmin",
    "networking",
    "hardware",
    "cloud",
    "server",
    "servers",
    "kubernetes",
    "docker",
    "linux",
    "security",
    "cybersecurity",
    "database",
    "architecture",
    "technical support",
    "helpdesk",
    "troubleshooting",
    "incident response",
    "observability",
    "devops",
  ],
  science: [
    "science",
    "scientific",
    "scientist",
    "physics",
    "chemistry",
    "biology",
    "experiment",
    "experiments",
    "hypothesis",
    "laboratory",
    "bioinformatics",
    "genomics",
    "climate",
    "empirical",
    "statistics",
    "research",
  ],
  translation: [
    "translate",
    "translation",
    "translator",
    "localize",
    "localise",
    "localization",
    "localisation",
    "multilingual",
    "bilingual",
    "interpreter",
    "subtitles",
    "transcreation",
    "spanish",
    "french",
    "german",
    "japanese",
    "mandarin",
    "portuguese",
  ],
  legal: [
    "legal",
    "law",
    "lawyer",
    "attorney",
    "counsel",
    "contract",
    "contracts",
    "compliance",
    "regulatory",
    "regulation",
    "litigation",
    "gdpr",
    "nda",
    "ndas",
    "paralegal",
    "statute",
    "clause",
    "clauses",
    "jurisdiction",
    "trademark",
    "patent",
    "intellectual property",
    "privacy policy",
    "terms of service",
    "due diligence",
  ],
  finance: [
    "finance",
    "financial",
    "accounting",
    "accountant",
    "bookkeeping",
    "invoice",
    "invoicing",
    "budget",
    "budgeting",
    "forecast",
    "forecasting",
    "revenue",
    "cash flow",
    "valuation",
    "investment",
    "investor",
    "portfolio",
    "tax",
    "taxes",
    "audit",
    "balance sheet",
    "treasury",
    "banking",
    "equities",
    "margin",
    "payroll",
  ],
  health: [
    "health",
    "medical",
    "medicine",
    "clinical",
    "patient",
    "patients",
    "diagnosis",
    "symptom",
    "symptoms",
    "nutrition",
    "therapy",
    "therapist",
    "wellness",
    "pharma",
    "pharmaceutical",
    "nurse",
    "physician",
    "fitness",
    "mental health",
    "triage",
  ],
  trivia: [
    "trivia",
    "quiz",
    "quizzes",
    "quizmaster",
    "general knowledge",
    "flashcard",
    "flashcards",
    "pub quiz",
    "recall",
  ],
  academia: [
    "academic",
    "academia",
    "thesis",
    "dissertation",
    "literature review",
    "citation",
    "citations",
    "peer review",
    "scholarly",
    "university",
    "professor",
    "tutoring",
    "tutor",
    "curriculum",
    "coursework",
    "lecture",
    "essay",
    "essays",
    "bibliography",
    "footnotes",
  ],
};

/**
 * Text as a run of space-separated words, padded at both ends.
 *
 * Padding is what makes a whole-word match a substring test: " law " finds the
 * word and not "lawn", "flawed" or "outlaw". Punctuation becomes a space rather
 * than nothing, so "contracts, compliance" is two words and "e-commerce" is
 * two rather than one nobody would match either way.
 */
function words(text: string): string {
  return ` ${text
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, " ")
    .trim()} `;
}

/** How much one field says about one use case. Each word counts once. */
function score(field: string, weight: number, vocabulary: string[]): number {
  let total = 0;
  for (const word of vocabulary) {
    if (field.includes(` ${word} `)) total += weight;
  }
  return total;
}

/**
 * What this agent reads as, if it reads as anything.
 *
 * `undefined` for the ordinary case: an agent whose work OpenRouter has no
 * category for. Also `undefined` for a tie, which is evidence pointing two ways
 * rather than a coin to toss — an agent described as both legal and finance
 * genuinely has no single best model, and saying so by staying quiet is better
 * than picking whichever list happens to be declared first.
 */
export function roleFor(evidence: Evidence): Role | undefined {
  const name = words(evidence.name);
  const skills = words(evidence.skills.join(" "));
  const instructions = words(evidence.instructions);

  let best: Role | undefined;
  let top = 0;
  let runnerUp = 0;

  for (const role of ROLES) {
    const vocabulary = WORDS[role.id] ?? [];
    const total =
      score(name, WEIGHT.name, vocabulary) +
      score(skills, WEIGHT.skills, vocabulary) +
      score(instructions, WEIGHT.instructions, vocabulary);

    if (total > top) {
      runnerUp = top;
      top = total;
      best = role;
    } else if (total > runnerUp) {
      runnerUp = total;
    }
  }

  if (top < FLOOR || top === runnerUp) return undefined;
  return best;
}
