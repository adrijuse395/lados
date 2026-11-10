package com.lados.desktop.autofill

import org.json.JSONArray
import java.util.concurrent.CopyOnWriteArrayList

/**
 * In-memory autofill credentials for the current unlocked session.
 * Cleared when the vault locks or the process dies — never written to disk.
 */
object AutofillVaultCache {
  data class Entry(
    val id: String,
    val title: String,
    val username: String,
    val password: String,
    val url: String?,
  )

  private val entries = CopyOnWriteArrayList<Entry>()

  @Volatile
  var unlocked: Boolean = false
    private set

  fun publish(json: String) {
    val parsed = ArrayList<Entry>()
    val array = JSONArray(json)
    for (i in 0 until array.length()) {
      val obj = array.getJSONObject(i)
      parsed.add(
        Entry(
          id = obj.optString("id"),
          title = obj.optString("title"),
          username = obj.optString("username"),
          password = obj.optString("password"),
          url = obj.optString("url").takeIf { it.isNotEmpty() },
        ),
      )
    }
    entries.clear()
    entries.addAll(parsed)
    unlocked = true
  }

  fun clear() {
    entries.clear()
    unlocked = false
  }

  fun matchesForHost(pageHost: String?): List<Entry> {
    if (!unlocked || pageHost.isNullOrBlank()) return emptyList()
    val page = normalizeHost(pageHost) ?: return emptyList()
    data class Ranked(val entry: Entry, val rank: Int)
    val ranked = ArrayList<Ranked>()
    for (entry in entries) {
      val entryHost = normalizeHost(hostOf(entry.url ?: continue) ?: continue) ?: continue
      val rank = when {
        page == entryHost -> 2
        page.endsWith(".$entryHost") -> 1
        else -> continue
      }
      ranked.add(Ranked(entry, rank))
    }
    ranked.sortWith(compareByDescending<Ranked> { it.rank }.thenBy { it.entry.title.lowercase() })
    return ranked.map { it.entry }
  }

  private fun hostOf(url: String): String? {
    val trimmed = url.trim()
    if (trimmed.isEmpty()) return null
    val rest = trimmed.substringAfter("://", trimmed)
    val authority = rest.split('/', '?', '#').firstOrNull() ?: return null
    val host = authority.substringAfterLast('@').substringBefore(':')
    return host.lowercase().takeIf { it.isNotEmpty() }
  }

  private fun normalizeHost(host: String): String? {
    val h = host.trim().lowercase()
    if (h.isEmpty()) return null
    return h.removePrefix("www.")
  }
}
