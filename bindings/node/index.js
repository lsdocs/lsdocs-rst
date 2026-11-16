'use strict'

// A fast reStructuredText parser building docutils' document tree. The
// tree is plain data (see index.d.ts); parsing, the writers and the
// settings are in Rust (lsdocs-rst), reached through native.js.

const native = require('./native.js')

const INFO = 1
const WARNING = 2
const ERROR = 3
const SEVERE = 4

/** Thrown by a directive or role: reported in the document as a system message. */
class DirectiveError extends Error {
  /**
   * @param {string} message
   * @param {number} [level] 1 info, 2 warning, 3 error (default), 4 severe
   */
  constructor(message, level = ERROR) {
    super(message)
    this.name = 'DirectiveError'
    this.level = level
  }
}

/** @param {any} node */
function isElement(node) {
  return typeof node === 'object' && node !== null && typeof node.kind === 'string'
}

/** @param {any} node */
function isText(node) {
  return typeof node === 'object' && node !== null && typeof node.text === 'string' && !('kind' in node)
}

/** @param {any} node @returns {any} */
function toNode(node) {
  if (typeof node === 'string') return { text: node }
  if (isElement(node) || isText(node)) return node
  throw new TypeError(`expected an element, text or string, not ${typeof node}`)
}

/** @param {any} node @returns {string} */
function astext(node) {
  if (isText(node)) return node.text
  return (node.children ?? []).map(astext).join('')
}

/** This element and its descendant elements, in document order. @param {any} element */
function* traverse(element) {
  const stack = [element]
  while (stack.length > 0) {
    const e = stack.pop()
    yield e
    const children = (e.children ?? []).filter(isElement)
    for (let i = children.length - 1; i >= 0; i--) stack.push(children[i])
  }
}

/** This element and its descendants of `kind` (all if omitted). @param {any} element @param {string} [kind] */
function findAll(element, kind) {
  return [...traverse(element)].filter((e) => kind === undefined || e.kind === kind)
}

/** @param {any} node */
function pformat(node) {
  return native.pformat(JSON.stringify(node))
}

/** @param {any} element @param {any} [options] */
function toXml(element, options = {}) {
  const xml = {
    newlines: options.newlines ?? false,
    indents: options.indents ?? false,
    xml_declaration: options.xmlDeclaration ?? true,
    doctype_declaration: options.doctypeDeclaration ?? true,
  }
  return native.toXml(JSON.stringify(element), JSON.stringify(xml))
}

/** @param {string} name */
function makeId(name) {
  return native.makeId(name)
}

function defaultSettings() {
  return JSON.parse(native.defaultSettings())
}

// -- extensions ------------------------------------------------------------

const OPTION_KINDS = new Set([
  'flag',
  'unchanged',
  'unchanged_required',
  'path',
  'uri',
  'int',
  'nonnegative_int',
  'percentage',
  'length_or_unitless',
  'length_or_percentage_or_unitless',
  'class_option',
])

/** @param {any} directive */
function spec(directive) {
  const options = Object.entries(directive.optionSpec ?? {}).map(([name, kind]) => {
    if (!Array.isArray(kind) && !OPTION_KINDS.has(kind)) {
      throw new TypeError(`unknown option kind ${JSON.stringify(kind)} for option "${name}"`)
    }
    return [name, kind]
  })
  return {
    required_arguments: directive.requiredArguments ?? 0,
    optional_arguments: directive.optionalArguments ?? 0,
    final_argument_whitespace: directive.finalArgumentWhitespace ?? false,
    has_content: directive.hasContent ?? false,
    options,
  }
}

/** @param {any} span */
function spanOf(span) {
  return span === null || span === undefined ? undefined : span
}

/** @param {string} kind @param {any} payload */
function ask(kind, payload) {
  const reply = JSON.parse(native.request(kind, JSON.stringify(payload)))
  if ('error' in reply) throw new Error(reply.error)
  return reply.ok
}

/** @param {any} data */
function directiveContext(data) {
  let alive = true
  /** @param {string} kind @param {any} payload */
  const live = (kind, payload) => {
    if (!alive) throw new Error('the directive has returned; its context can no longer parse')
    return ask(kind, payload)
  }
  const ctx = {
    name: data.name,
    arguments: data.arguments,
    options: data.options,
    content: data.content,
    lineno: data.lineno,
    blockText: data.block_text,
    span: spanOf(data.span),
    inSubstitution: data.in_substitution,
    /** @param {any} container */
    parseContent: (container) => live('parse_content', container),
    /** @param {string} text */
    parseInline: (text) => live('parse_inline', text),
    /** @param {number} level @param {string} text */
    message: (level, text) => live('message', [level, text]),
  }
  return { ctx, close: () => { alive = false } }
}

/** @param {any} data */
function roleContext(data) {
  return {
    name: data.name,
    text: data.text,
    rawText: data.raw_text,
    rawsource: data.rawsource,
    options: data.options,
    lineno: data.lineno,
    span: spanOf(data.span),
  }
}

/**
 * Runs `fn`, turning its result or exception into the outcome the addon
 * expects. Exceptions other than DirectiveError are kept in `thrown`.
 * @param {() => any[]} fn @param {unknown[]} thrown
 */
function outcome(fn, thrown) {
  try {
    return ['ok', fn().map(toNode)]
  } catch (error) {
    if (error instanceof DirectiveError) return ['error', error.level, error.message]
    thrown.push(error)
    return ['throw']
  }
}

/** @param {any} loader @param {string} arg @param {unknown[]} thrown */
function load(loader, arg, thrown) {
  try {
    const data = loader(arg)
    const buffer = typeof data === 'string' ? Buffer.from(data, 'utf8') : Buffer.from(data)
    return ['ok', buffer.toString('latin1')]
  } catch (error) {
    const e = /** @type {any} */ (error)
    if (e && typeof e.code === 'string' && e.code.startsWith('E')) {
      return ['oserror', e.code, MESSAGES[e.code] ?? e.message]
    }
    thrown.push(error)
    return ['throw']
  }
}

/** Python's wording for the errors docutils reports. @type {Record<string, string>} */
const MESSAGES = {
  ENOENT: 'No such file or directory',
  EACCES: 'Permission denied',
  EPERM: 'Operation not permitted',
  EISDIR: 'Is a directory',
}

// -- parsing ---------------------------------------------------------------

class Parser {
  /** @param {any} [options] */
  constructor(options = {}) {
    const known = new Set([
      'source', 'transforms', 'keepUnknownDirectives', 'settings',
      'directives', 'roles', 'root', 'files', 'urls',
    ])
    for (const key of Object.keys(options)) {
      if (!known.has(key)) throw new TypeError(`unknown parser option "${key}"`)
    }
    if (options.root !== undefined && (options.files || options.urls)) {
      throw new TypeError('give a root directory or loader functions, not both')
    }
    this.options = { ...options }
    this.directives = { ...(options.directives ?? {}) }
    this.roles = { ...(options.roles ?? {}) }
    // Checked now, so a bad option spec fails early.
    this.specs = Object.entries(this.directives).map(([name, d]) => [name, spec(d)])
  }

  /** The docutils settings in use. */
  get settings() {
    return { ...defaultSettings(), ...(this.options.settings ?? {}) }
  }

  /** @param {string} text */
  parse(text) {
    return this.parseWithMessages(text).document
  }

  /** @param {string} text */
  parseWithMessages(text) {
    const o = this.options
    const options = {
      source: o.source ?? null,
      transforms: o.transforms ?? false,
      keep_unknown_directives: o.keepUnknownDirectives ?? false,
      settings: o.settings ?? null,
      directives: this.specs,
      roles: Object.keys(this.roles),
      root: o.root === undefined ? null : String(o.root),
      files: typeof o.files === 'function',
      urls: typeof o.urls === 'function',
    }
    /** @type {unknown[]} */
    const thrown = []
    /** @param {string} json */
    const dispatch = (json) => {
      const call = JSON.parse(json)
      let answer
      if (call.kind === 'directive') {
        const directive = this.directives[call.name] ?? this.directives[lookup(this.directives, call.name)]
        const { ctx, close } = directiveContext(call.context)
        answer = outcome(() => directive.run(ctx), thrown)
        close()
      } else if (call.kind === 'role') {
        const role = this.roles[call.name] ?? this.roles[lookup(this.roles, call.name)]
        answer = outcome(() => role(roleContext(call.context)), thrown)
      } else {
        answer = load(call.kind === 'file' ? o.files : o.urls, call.name, thrown)
      }
      return JSON.stringify(answer)
    }
    const [document, messages] = JSON.parse(native.parse(text, JSON.stringify(options), dispatch))
    if (thrown.length > 0) throw thrown[0]
    return { document, messages }
  }
}

/** Directive and role names match case-insensitively. @param {Record<string, any>} table @param {string} name */
function lookup(table, name) {
  const lower = name.toLowerCase()
  return Object.keys(table).find((key) => key.toLowerCase() === lower) ?? name
}

/** @param {string} text @param {any} [options] */
function parse(text, options) {
  return new Parser(options).parse(text)
}

/** @param {string} text @param {any} [options] */
function parseWithMessages(text, options) {
  return new Parser(options).parseWithMessages(text)
}

module.exports = {
  DirectiveError,
  Parser,
  INFO,
  WARNING,
  ERROR,
  SEVERE,
  astext,
  defaultSettings,
  findAll,
  isElement,
  isText,
  makeId,
  parse,
  parseWithMessages,
  pformat,
  toXml,
  traverse,
}
