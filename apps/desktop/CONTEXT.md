# Desktop

The Desktop context owns Nest's local knowledge workspace and the user-facing chat experience over that workspace.

## Language

**Chat Backend**:
The runtime that produces a chat response and conducts any model-driven tool loop. Nest Agent and Claude Agent are Chat Backends.
_Avoid_: Provider, engine, chat mode

**Chat Mode**:
The per-turn policy applied to Nest Knowledge Capabilities. Ask permits read-only knowledge operations; Agent also permits reviewable Knowledge Change Proposals. A Chat Backend may declare a separate native tool surface, such as Claude Agent's Bash and file tools.
_Avoid_: Backend, provider, agent selector

**Nest Agent**:
The built-in Chat Backend implemented with Rig and an OpenAI-compatible model endpoint.
_Avoid_: Default provider, OpenAI mode

**Claude Agent**:
The external Chat Backend implemented by invoking an authenticated Claude CLI installation.
_Avoid_: Claude mode, Claude provider

**Knowledge Capability**:
A Nest-controlled operation that lets a Chat Backend retrieve from or propose changes to the local knowledge workspace. Claude Agent receives Knowledge Capabilities only through an explicit Nest integration.
_Avoid_: Native Claude tool, direct vault access

**Capability Catalog**:
The authoritative set of Knowledge Capabilities available for a turn, including their stable identities, schemas, permitted Chat Modes, and effects. Chat Backends consume the catalog rather than defining their own knowledge permissions.
_Avoid_: Claude tool list, MCP configuration

**Knowledge Change Proposal**:
A per-file change produced in turn-local staging during a successful Agent turn. It changes the knowledge workspace only after the user approves it through Nest's review flow. If its disk baseline changes, Nest first attempts a deterministic three-way rebase; only an unmergeable overlap becomes conflicted and cannot be approved.
_Avoid_: Tool approval, direct agent write

**Open Claude Runtime**:
The Claude Agent execution model in which Agent turns retain Claude CLI's native tools while Nest adds governed Domain Capabilities. Nest does not claim that native Bash, Edit, or Write operations pass through its proposal review.
_Avoid_: Governed Claude Runtime, Nest-only sandbox

**Direct Workspace Change**:
A Vault file change made through a Chat Backend's native tools or another external process rather than a Nest Knowledge Capability. Nest may observe and re-index it, but it is not a Knowledge Change Proposal and must not be presented as reviewed.
_Avoid_: Proposal, approved change, staged change

**Tool Activity**:
A normalized, bounded chat event describing a native Backend tool or Nest Domain Capability invocation and its lifecycle. It may identify direct workspace access, but it is neither a permission approval nor proof that a file change was reviewed.
_Avoid_: Raw protocol event, Knowledge Change Proposal, audit log

**Chat Turn**:
The durable execution started by one sent user message. It snapshots Backend, Model, and Chat Mode and owns Tool Activities even when no assistant message is produced because execution fails, is cancelled, or is interrupted.
_Avoid_: Chat session, assistant message, Claude transcript

**Vault Reconciliation**:
The bounded scan run after each Claude Chat Turn and during recovery to detect direct active-pack Markdown changes, rebase or conflict stale proposals, and update the Nest index. Step 2 does not provide a continuous filesystem watcher.
_Avoid_: File watcher, Knowledge Change Proposal, live sync

**Reindex Required**:
A persistent degraded state entered when Vault Reconciliation cannot establish a trustworthy active-pack manifest and index. Nest Knowledge Capabilities remain unavailable until Reindex and proposal reconciliation succeed; Claude native tools may remain usable.
_Avoid_: Backend disconnected, Claude unavailable

**Nest Domain Capability**:
A Nest-controlled operation with business semantics, validation, authorization, state transitions, and structured errors. Knowledge Capabilities are the Step 2 subset; future examples include Pack synchronization, Pack publication, and Hub operations.
_Avoid_: Bash command, Claude native tool

**Nest-first Routing**:
The Claude Agent behavior contract that prefers Nest Knowledge Capabilities for active-pack Markdown so reads produce governed citations and writes produce reviewable proposals. Native tools remain available as a fallback when the user explicitly requests direct access or the Capability Catalog cannot express the task.
_Avoid_: Sandbox, mandatory tool interception, native-tool ban

**LLM Wiki**:
A future project-scoped knowledge system that ingests Vault documents into a retrievable, source-grounded knowledge base for Chat Backends. It is not part of Step 2.
_Avoid_: Generated documentation site, chat transcript archive

**Project Skill**:
A future Claude Agent Skill maintained inside a Vault or the Nest project root and made available to Claude when activated. Project Skill discovery, activation, packaging, and execution are not part of Step 2.
_Avoid_: Knowledge Capability, global Codex skill

## Delivery Stages

**Step 1**:
The Windows-only delivery stage in which an enabled Claude Agent handles chat and a disabled Claude Agent leaves chat on Nest Agent. It adds configuration and connection verification but no Knowledge Capabilities or automatic Claude titles; Ask and Agent remain visible but have identical behavior on Claude Agent.

**Step 2**:
The delivery stage that adds Chat Backend and model selection and exposes Nest Knowledge Capabilities to Claude Agent.

**Claude Session Binding**:
The one-to-one association in which a Nest chat session UUID is also the Claude session ID used to resume its conversation. A bound Nest chat never changes to a different Claude session.
_Avoid_: Continue latest session, shared Claude session

**Backend Binding**:
The Chat Backend chosen atomically when the first message is sent in an unbound Nest chat session. A bound session keeps its Chat Backend; changes to the enabled default apply only to new, unbound chats.
_Avoid_: Per-message routing, implicit backend switching

**Backend Selection**:
The provisional Chat Backend choice for an unbound chat. The first sent message turns it into an immutable Backend Binding; choosing another Backend for a bound chat starts a new chat.
_Avoid_: Backend switching, per-message routing

**Backend Descriptor**:
The registered description of a Chat Backend: its stable identity, availability, model choices, supported Chat Modes, and capability profile. The chat UI derives Backend choices from descriptors.
_Avoid_: Provider settings, frontend option

**Model Selection**:
The model choice within a Chat Backend. Claude Agent may change Model Selection between turns without changing its Backend Binding; Nest Agent uses its currently configured model.
_Avoid_: Model Binding, Chat Backend

**Observed Model**:
A non-empty effective model ID reported by a successful Claude connection test or successful Claude Chat Turn. Observed Models are projected into Claude Agent's model options but do not rewrite Custom Models or the current Model Selection.
_Avoid_: Auto-discovered account model, Custom Model, Model Selection
