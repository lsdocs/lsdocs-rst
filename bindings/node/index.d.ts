// Types for @lsdocs/rst. The tree is plain data, the same form as the Rust
// crate's serde feature, so it round-trips through JSON.

/** Byte offsets in the parsed text: `start` inclusive, `end` exclusive. */
export interface Span {
  start: number
  end: number
}

/** An attribute value. */
export type AttrValue = string | string[] | number | boolean | null

/** An element: a docutils node kind (`"paragraph"`, `"section"`, ...). */
export interface Element {
  kind: string
  attributes?: Record<string, AttrValue>
  children?: Node[]
  span?: Span
}

/** Text in the tree. */
export interface Text {
  text: string
  span?: Span
}

export type Node = Element | Text

/** The docutils settings the parser honors, by their docutils names. */
export interface Settings {
  doctitle_xform?: boolean
  docinfo_xform?: boolean
  sectsubtitle_xform?: boolean
  sectnum_xform?: boolean
  title?: string | null
  tab_width?: number
  line_length_limit?: number
  raw_enabled?: boolean
  pep_references?: boolean
  pep_base_url?: string
  pep_file_url_template?: string
  rfc_references?: boolean
  rfc_base_url?: string
  trim_footnote_reference_space?: boolean
  toc_backlinks?: 'entry' | 'top' | 'none'
  id_prefix?: string
  auto_id_prefix?: string
  strip_comments?: boolean
  /** Task lists (`- [ ] item`, `- [x] item`); not a docutils setting. Off by default. */
  task_lists?: boolean
}

/** docutils' option conversion functions, by name. */
export type OptionKind =
  | 'flag'
  | 'unchanged'
  | 'unchanged_required'
  | 'path'
  | 'uri'
  | 'int'
  | 'nonnegative_int'
  | 'percentage'
  | 'length_or_unitless'
  | 'length_or_percentage_or_unitless'
  | 'class_option'

/** A converted option value: `null` for flags. */
export type OptionValue = string | number | string[] | null

/** What a directive's `run` gets. */
export interface DirectiveContext {
  /** The directive name as written. */
  readonly name: string
  readonly arguments: string[]
  readonly options: Record<string, OptionValue>
  /** The content lines, indentation removed. */
  readonly content: string[]
  /** The line number of the directive's first line. */
  readonly lineno: number
  readonly blockText: string
  readonly span?: Span
  readonly inSubstitution: boolean
  /** Parses the content as body elements into `container`. */
  parseContent(container: Element): Element
  /** Parses `text` as inline markup: the nodes, and messages to put after their element. */
  parseInline(text: string): [Node[], Element[]]
  /** A system message (1 info to 4 severe) for the directive's line. */
  message(level: number, text: string): Element
}

/** A directive: what it accepts, and `run`. */
export interface Directive {
  requiredArguments?: number
  optionalArguments?: number
  finalArgumentWhitespace?: boolean
  /** Option names to conversion functions, or to lists of choices. */
  optionSpec?: Record<string, OptionKind | string[]>
  hasContent?: boolean
  /** The directive's nodes; throw a `DirectiveError` to report a problem instead. */
  run(ctx: DirectiveContext): Array<Node | string>
}

/** What a role function gets. */
export interface RoleContext {
  readonly name: string
  /** The text, escapes removed. */
  readonly text: string
  /** The text with its backslashes, as written. */
  readonly rawText: string
  /** The whole interpreted text as written, role included. */
  readonly rawsource: string
  /** Options from the `role` directive that made the role. */
  readonly options: Record<string, OptionValue>
  readonly lineno: number
  readonly span?: Span
}

export type RoleFunction = (ctx: RoleContext) => Array<Node | string>

/** Reads a file or URL: bytes, or a string (taken as UTF-8). Throw a Node.js
 * system error (`code` such as `ENOENT`) for a failed read. */
export type Loader = (path: string) => Uint8Array | string

export interface ParserOptions {
  /** The document's name, used in messages. */
  source?: string
  /** Apply docutils' transforms, as `publish_doctree` does. */
  transforms?: boolean
  /** Keep unknown directives as `directive` elements instead of reporting them. */
  keepUnknownDirectives?: boolean
  settings?: Settings
  /** Directives by name, before any standard one of that name. */
  directives?: Record<string, Directive>
  /** Role functions by name. */
  roles?: Record<string, RoleFunction>
  /** Let directives read files below this directory. */
  root?: string
  /** Or read files with this function. */
  files?: Loader
  /** And URLs with this one. */
  urls?: Loader
}

export interface ParseResult {
  document: Element
  /** Messages docutils' transforms report outside the tree. */
  messages: Element[]
}

/** Thrown by a directive or role: reported in the document as a system message. */
export class DirectiveError extends Error {
  constructor(message: string, level?: number)
  readonly level: number
}

export class Parser {
  constructor(options?: ParserOptions)
  /** The docutils settings in use. */
  readonly settings: Required<Settings>
  /** Parses `text` into a `document` element. Throws only when a JS
   * directive, role or loader throws (other than `DirectiveError` and
   * system errors). */
  parse(text: string): Element
  parseWithMessages(text: string): ParseResult
}

export const INFO: 1
export const WARNING: 2
export const ERROR: 3
export const SEVERE: 4

export function parse(text: string, options?: ParserOptions): Element
export function parseWithMessages(text: string, options?: ParserOptions): ParseResult
/** Pseudo-XML, exactly as docutils' `pformat()` prints it. */
export function pformat(node: Node): string
/** Docutils XML, as `docutils --writer=xml` writes it. */
export function toXml(
  element: Element,
  options?: { newlines?: boolean; indents?: boolean; xmlDeclaration?: boolean; doctypeDeclaration?: boolean },
): string
export function astext(node: Node): string
/** This element and its descendant elements, in document order. */
export function traverse(element: Element): Generator<Element>
/** This element and its descendants of `kind` (all if omitted). */
export function findAll(element: Element, kind?: string): Element[]
export function isElement(node: unknown): node is Element
export function isText(node: unknown): node is Text
/** `name` as docutils makes ids of names. */
export function makeId(name: string): string
/** The settings with docutils' defaults. */
export function defaultSettings(): Required<Settings>
