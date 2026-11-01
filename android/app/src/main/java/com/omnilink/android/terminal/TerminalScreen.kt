package com.omnilink.android.terminal

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import kotlin.math.max
import kotlin.math.min

data class StyledChar(
    val char: Char = ' ',
    val color: Color = Color(0xFFE2E8F0),
    val isBold: Boolean = false
)

class TerminalRow(val maxCols: Int = 120) {
    private val chars = ArrayList<StyledChar>()

    fun setChar(col: Int, sc: StyledChar) {
        if (col < 0 || col >= maxCols) return
        while (chars.size <= col) {
            chars.add(StyledChar(' ', sc.color, false))
        }
        chars[col] = sc
    }

    fun clearToEnd(fromCol: Int) {
        if (fromCol < chars.size) {
            val toKeep = max(0, fromCol)
            while (chars.size > toKeep) {
                chars.removeAt(chars.size - 1)
            }
        }
    }

    fun clearFromStart(toCol: Int) {
        val end = min(chars.size, toCol + 1)
        for (i in 0 until end) {
            chars[i] = StyledChar(' ', Color(0xFFE2E8F0), false)
        }
    }

    fun clearAll() {
        chars.clear()
    }

    fun toAnnotatedString(defaultColor: Color): AnnotatedString {
        if (chars.isEmpty()) return AnnotatedString("")

        var lastNonSpace = chars.size - 1
        while (lastNonSpace >= 0 && chars[lastNonSpace].char == ' ') {
            lastNonSpace--
        }
        if (lastNonSpace < 0) return AnnotatedString("")

        return buildAnnotatedString {
            var i = 0
            while (i <= lastNonSpace) {
                val sc = chars[i]
                val start = i
                val curColor = sc.color
                val curBold = sc.isBold

                while (i <= lastNonSpace && chars[i].color == curColor && chars[i].isBold == curBold) {
                    append(chars[i].char)
                    i++
                }

                val end = i
                addStyle(
                    SpanStyle(
                        color = curColor,
                        fontWeight = if (curBold) FontWeight.Bold else FontWeight.Normal
                    ),
                    start,
                    end
                )
            }
        }
    }
}

class TerminalScreen(
    val maxScrollback: Int = 800,
    val defaultCols: Int = 100,
    val defaultColor: Color = Color(0xFF38BDF8)
) {
    var renderedLines by mutableStateOf<List<AnnotatedString>>(emptyList())
        private set

    private val scrollback = ArrayList<AnnotatedString>()
    private val activeRows = ArrayList<TerminalRow>()
    private val pendingBuffer = StringBuilder()

    private var cursorRow = 0
    private var cursorCol = 0
    private var currentColor = defaultColor
    private var isBold = false

    private val ansiColorMap = mapOf(
        0 to Color(0xFF1E2230),  // Black
        1 to Color(0xFFEF4444),  // Red
        2 to Color(0xFF10B981),  // Green
        3 to Color(0xFFF59E0B),  // Yellow
        4 to Color(0xFF3B82F6),  // Blue
        5 to Color(0xFFA855F7),  // Magenta
        6 to Color(0xFF06B6D4),  // Cyan
        7 to Color(0xFFE2E8F0),  // White

        // Bright variants (90-97)
        8 to Color(0xFF94A3B8),  // Bright Black / Gray
        9 to Color(0xFFF87171),  // Bright Red
        10 to Color(0xFF34D399), // Bright Green
        11 to Color(0xFFFDE047), // Bright Yellow
        12 to Color(0xFF60A5FA), // Bright Blue
        13 to Color(0xFFC084FC), // Bright Magenta
        14 to Color(0xFF22D3EE), // Bright Cyan
        15 to Color(0xFFFFFFFF)  // Bright White
    )

    @Synchronized
    fun feed(rawText: String) {
        val text = if (pendingBuffer.isNotEmpty()) {
            val combined = pendingBuffer.toString() + rawText
            pendingBuffer.clear()
            combined
        } else {
            rawText
        }

        var idx = 0
        val len = text.length

        while (idx < len) {
            val ch = text[idx]

            when {
                ch == '\u001b' -> {
                    val escapeStart = idx
                    idx++
                    if (idx >= len) {
                        // Partial escape at end of stream: buffer for next chunk
                        pendingBuffer.append(text.substring(escapeStart))
                        break
                    }
                    val next = text[idx]
                    when (next) {
                        '[' -> {
                            // CSI sequence: parse parameters and command character in '@'..'~'
                            idx++
                            val sb = StringBuilder()
                            var foundTerminator = false
                            while (idx < len) {
                                val c = text[idx]
                                if (c in '@'..'~') {
                                    handleCsi(sb.toString(), c)
                                    idx++
                                    foundTerminator = true
                                    break
                                } else {
                                    sb.append(c)
                                    idx++
                                }
                            }
                            if (!foundTerminator) {
                                // Incomplete CSI fragment: buffer for next chunk
                                pendingBuffer.append(text.substring(escapeStart))
                                break
                            }
                        }
                        ']' -> {
                            // OSC sequence: skip until BEL (\u0007) or ST (\u001b\)
                            idx++
                            var foundEnd = false
                            while (idx < len) {
                                if (text[idx] == '\u0007') {
                                    idx++
                                    foundEnd = true
                                    break
                                } else if (text[idx] == '\u001b' && idx + 1 < len && text[idx + 1] == '\\') {
                                    idx += 2
                                    foundEnd = true
                                    break
                                }
                                idx++
                            }
                            if (!foundEnd) {
                                pendingBuffer.append(text.substring(escapeStart))
                                break
                            }
                        }
                        '(', ')', '*', '+' -> {
                            // Character set switching: 2-byte sequence
                            if (idx + 1 < len) {
                                idx += 2
                            } else {
                                pendingBuffer.append(text.substring(escapeStart))
                                break
                            }
                        }
                        else -> {
                            idx++
                        }
                    }
                }
                ch == '\r' -> {
                    cursorCol = 0
                    idx++
                }
                ch == '\n' -> {
                    cursorRow++
                    ensureActiveRow(cursorRow)
                    idx++
                }
                ch == '\b' -> {
                    cursorCol = max(0, cursorCol - 1)
                    idx++
                }
                ch == '\t' -> {
                    cursorCol = (cursorCol / 8 + 1) * 8
                    idx++
                }
                ch < ' ' -> {
                    // Control chars like \u0007 (bell), etc.
                    idx++
                }
                else -> {
                    ensureActiveRow(cursorRow)
                    activeRows[cursorRow].setChar(
                        cursorCol,
                        StyledChar(ch, currentColor, isBold)
                    )
                    cursorCol++
                    idx++
                }
            }
        }

        rebuildRenderedLines()
    }

    private fun ensureActiveRow(row: Int) {
        while (activeRows.size <= row) {
            activeRows.add(TerminalRow(defaultCols))
        }
    }

    private fun handleCsi(params: String, cmd: Char) {
        when (cmd) {
            'm' -> {
                handleSgr(params)
            }
            'H', 'f' -> {
                val parts = params.split(';').mapNotNull { it.toIntOrNull() }
                val targetRow = if (parts.isNotEmpty()) parts[0] - 1 else 0
                val targetCol = if (parts.size > 1) parts[1] - 1 else 0
                cursorRow = max(0, targetRow)
                cursorCol = max(0, targetCol)
                ensureActiveRow(cursorRow)
            }
            'A' -> {
                val n = params.filter { it.isDigit() }.toIntOrNull() ?: 1
                cursorRow = max(0, cursorRow - n)
            }
            'B' -> {
                val n = params.filter { it.isDigit() }.toIntOrNull() ?: 1
                cursorRow += n
                ensureActiveRow(cursorRow)
            }
            'C' -> {
                val n = params.filter { it.isDigit() }.toIntOrNull() ?: 1
                cursorCol += n
            }
            'D' -> {
                val n = params.filter { it.isDigit() }.toIntOrNull() ?: 1
                cursorCol = max(0, cursorCol - n)
            }
            'G' -> {
                val col = params.filter { it.isDigit() }.toIntOrNull() ?: 1
                cursorCol = max(0, col - 1)
            }
            'd' -> {
                val row = params.filter { it.isDigit() }.toIntOrNull() ?: 1
                cursorRow = max(0, row - 1)
                ensureActiveRow(cursorRow)
            }
            'J' -> {
                val p = params.filter { it.isDigit() }.toIntOrNull() ?: 0
                if (p == 2 || p == 3) {
                    for (row in activeRows) {
                        val astr = row.toAnnotatedString(defaultColor)
                        if (astr.isNotEmpty()) {
                            scrollback.add(astr)
                        }
                    }
                    while (scrollback.size > maxScrollback) {
                        scrollback.removeAt(0)
                    }
                    activeRows.clear()
                    cursorRow = 0
                    cursorCol = 0
                }
            }
            'K' -> {
                val p = params.filter { it.isDigit() }.toIntOrNull() ?: 0
                if (cursorRow < activeRows.size) {
                    when (p) {
                        0 -> activeRows[cursorRow].clearToEnd(cursorCol)
                        1 -> activeRows[cursorRow].clearFromStart(cursorCol)
                        2 -> activeRows[cursorRow].clearAll()
                    }
                }
            }
            'X' -> {
                // Erase character without moving cursor
                val n = params.filter { it.isDigit() }.toIntOrNull() ?: 1
                if (cursorRow < activeRows.size) {
                    for (i in 0 until n) {
                        activeRows[cursorRow].setChar(cursorCol + i, StyledChar(' ', currentColor, false))
                    }
                }
            }
            else -> {}
        }
    }

    private fun handleSgr(params: String) {
        if (params.isEmpty()) {
            currentColor = defaultColor
            isBold = false
            return
        }

        val tokens = params.split(';').mapNotNull { it.toIntOrNull() }
        var i = 0
        while (i < tokens.size) {
            when (val code = tokens[i]) {
                0 -> {
                    currentColor = defaultColor
                    isBold = false
                }
                1 -> isBold = true
                22 -> isBold = false
                in 30..37 -> {
                    currentColor = ansiColorMap[code - 30] ?: defaultColor
                }
                39 -> currentColor = defaultColor
                in 90..97 -> {
                    currentColor = ansiColorMap[code - 90 + 8] ?: defaultColor
                }
                38 -> {
                    if (i + 1 < tokens.size) {
                        when (tokens[i + 1]) {
                            2 -> {
                                if (i + 4 < tokens.size) {
                                    val r = tokens[i + 2].coerceIn(0, 255)
                                    val g = tokens[i + 3].coerceIn(0, 255)
                                    val b = tokens[i + 4].coerceIn(0, 255)
                                    currentColor = Color(r, g, b)
                                    i += 4
                                }
                            }
                            5 -> {
                                if (i + 2 < tokens.size) {
                                    val n = tokens[i + 2]
                                    currentColor = ansi256Color(n)
                                    i += 2
                                }
                            }
                        }
                    }
                }
            }
            i++
        }
    }

    private fun ansi256Color(n: Int): Color {
        return when {
            n in 0..7 -> ansiColorMap[n] ?: defaultColor
            n in 8..15 -> ansiColorMap[n] ?: defaultColor
            n in 16..231 -> {
                val num = n - 16
                val b = (num % 6) * 51
                val g = ((num / 6) % 6) * 51
                val r = (num / 36) * 51
                Color(r, g, b)
            }
            n in 232..255 -> {
                val gray = (n - 232) * 10 + 8
                Color(gray, gray, gray)
            }
            else -> defaultColor
        }
    }

    private fun rebuildRenderedLines() {
        val result = ArrayList<AnnotatedString>(scrollback.size + activeRows.size)
        result.addAll(scrollback)
        for (row in activeRows) {
            result.add(row.toAnnotatedString(defaultColor))
        }
        renderedLines = result
    }

    fun clear() {
        pendingBuffer.clear()
        scrollback.clear()
        activeRows.clear()
        cursorRow = 0
        cursorCol = 0
        renderedLines = emptyList()
    }
}
