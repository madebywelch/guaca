export interface Role {
  id: string;
  label: string;
}

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

export interface Evidence {
  name: string;
  skills: string[];
  instructions: string;
}

const WEIGHT = { name: 5, skills: 4, instructions: 2 } as const;

const FLOOR = 4;

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

function words(text: string): string {
  return ` ${text
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, " ")
    .trim()} `;
}

function score(field: string, weight: number, vocabulary: string[]): number {
  let total = 0;
  for (const word of vocabulary) {
    if (field.includes(` ${word} `)) total += weight;
  }
  return total;
}

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
