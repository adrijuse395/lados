package com.lados.desktop.autofill

import android.app.assist.AssistStructure
import android.os.Build
import android.view.View
import android.view.autofill.AutofillId
import java.util.Locale

data class ParsedField(
  val id: AutofillId,
  val hint: String,
)

data class ParsedStructure(
  val webDomain: String?,
  val usernameIds: List<AutofillId>,
  val passwordIds: List<AutofillId>,
)

object AssistStructureParser {
  fun parse(structure: AssistStructure): ParsedStructure {
    val usernames = ArrayList<AutofillId>()
    val passwords = ArrayList<AutofillId>()
    var webDomain: String? = null

    for (i in 0 until structure.windowNodeCount) {
      val root = structure.getWindowNodeAt(i).rootViewNode
      webDomain = webDomain ?: findWebDomain(root)
      walk(root, usernames, passwords)
    }

    return ParsedStructure(webDomain, usernames, passwords)
  }

  private fun findWebDomain(node: AssistStructure.ViewNode): String? {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
      node.webDomain?.takeIf { it.isNotBlank() }?.let { return it.lowercase(Locale.US) }
    }
    for (i in 0 until node.childCount) {
      findWebDomain(node.getChildAt(i))?.let { return it }
    }
    return null
  }

  private fun walk(
    node: AssistStructure.ViewNode,
    usernames: MutableList<AutofillId>,
    passwords: MutableList<AutofillId>,
  ) {
    val id = node.autofillId
    if (id != null && node.autofillType != View.AUTOFILL_TYPE_NONE) {
      when (classify(node)) {
        FieldKind.Username -> usernames.add(id)
        FieldKind.Password -> passwords.add(id)
        FieldKind.Other -> Unit
      }
    }
    for (i in 0 until node.childCount) {
      walk(node.getChildAt(i), usernames, passwords)
    }
  }

  private enum class FieldKind { Username, Password, Other }

  private fun classify(node: AssistStructure.ViewNode): FieldKind {
    val hints = node.autofillHints?.joinToString(" ")?.lowercase(Locale.US).orEmpty()
    if (hints.contains("password") || hints.contains(View.AUTOFILL_HINT_PASSWORD)) {
      return FieldKind.Password
    }
    if (
      hints.contains("username") ||
      hints.contains("email") ||
      hints.contains(View.AUTOFILL_HINT_USERNAME) ||
      hints.contains(View.AUTOFILL_HINT_EMAIL_ADDRESS)
    ) {
      return FieldKind.Username
    }

    val input = node.htmlInfo?.attributes
      ?.firstOrNull { it.first.equals("type", ignoreCase = true) }
      ?.second
      ?.lowercase(Locale.US)
      .orEmpty()
    if (input == "password") return FieldKind.Password
    if (input == "email" || input == "username") return FieldKind.Username

    if (node.inputType and android.text.InputType.TYPE_TEXT_VARIATION_PASSWORD != 0 ||
      node.inputType and android.text.InputType.TYPE_NUMBER_VARIATION_PASSWORD != 0 ||
      node.inputType and android.text.InputType.TYPE_TEXT_VARIATION_WEB_PASSWORD != 0
    ) {
      return FieldKind.Password
    }

    val bag = listOfNotNull(
      node.idEntry,
      node.hint,
      node.contentDescription?.toString(),
      node.text?.toString(),
    ).joinToString(" ").lowercase(Locale.US)

    if (bag.contains("password") || bag.contains("passwd") || bag.contains("passwort")) {
      return FieldKind.Password
    }
    if (
      bag.contains("username") ||
      bag.contains("user name") ||
      bag.contains("email") ||
      bag.contains("e-mail") ||
      bag.contains("login")
    ) {
      return FieldKind.Username
    }
    return FieldKind.Other
  }
}
