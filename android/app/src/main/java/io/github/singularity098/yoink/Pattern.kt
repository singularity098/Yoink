package io.github.singularity098.yoink

/**
 * File name patterns, as in the desktop app (src/pattern.rs). An app
 * remembers which release asset or Actions artifact it uses as a glob (`*`
 * and `?`), made from the name picked once with its version numbers and
 * commit hashes turned into `*`, so the next version's file matches too.
 */
object Pattern {
    /** Whether [text] matches the glob [pattern]. Case matters. */
    fun matches(pattern: String, text: String): Boolean {
        val p = pattern
        val t = text
        var pi = 0
        var ti = 0
        var starP = -1
        var starT = -1
        while (ti < t.length) {
            if (pi < p.length && (p[pi] == '?' || p[pi] == t[ti])) {
                pi++; ti++
            } else if (pi < p.length && p[pi] == '*') {
                starP = pi; starT = ti; pi++
            } else if (starP >= 0) {
                pi = starP + 1; starT++; ti = starT
            } else {
                return false
            }
        }
        while (pi < p.length && p[pi] == '*') pi++
        return pi == p.length
    }

    private val VERSION = Regex(
        """(?i)\d+(?:\.\d+)+(?:[-+.~_]?(?:alpha|beta|rc|dev|pre|nightly|snapshot)[.-]?\d*)?"""
    )

    // Commit hashes and dates: runs of 7 to 40 hex digits (with at least one
    // digit) or 8-digit dates, standing apart from other letters and digits.
    private val HASH = Regex("""(^|[^0-9A-Za-z])([0-9a-f]{7,40}|\d{8})([^0-9A-Za-z]|$)""")

    private const val MARK = '\u0000'

    /**
     * A pattern for [name] that the next version's file is likely to match:
     * the version ([tag], with or without its `v`), other dotted version
     * numbers, commit hashes and dates become `*`.
     */
    fun derive(name: String, tag: String? = null): String {
        var s = name
        if (tag != null) {
            val bare = tag.trimStart('v', 'V')
            // A tag that is only a word ("nightly") is left: the next one has the same name.
            if (bare.any { it.isDigit() } && bare.length >= 2) {
                s = s.replace(tag, MARK.toString()).replace(bare, MARK.toString())
            }
        }
        s = VERSION.replace(s, MARK.toString())
        // Hashes need a loop: neighbouring matches share their separators.
        while (true) {
            val next = HASH.replace(s) { m ->
                val h = m.groupValues[2]
                val digitsOnly = h.all { it.isDigit() }
                val hasDigit = h.any { it.isDigit() }
                if ((digitsOnly && h.length != 8) || !hasDigit) m.value
                else m.groupValues[1] + MARK + m.groupValues[3]
            }
            if (next == s) break
            s = next
        }
        val out = StringBuilder()
        for (ch in s) {
            val c = if (ch == MARK) '*' else ch
            if (c == '*' && out.endsWith("*")) continue
            out.append(c)
        }
        return out.toString()
    }
}
