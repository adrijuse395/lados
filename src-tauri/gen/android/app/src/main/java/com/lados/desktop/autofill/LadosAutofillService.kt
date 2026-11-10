package com.lados.desktop.autofill

import android.app.PendingIntent
import android.app.assist.AssistStructure
import android.content.Intent
import android.os.CancellationSignal
import android.service.autofill.Dataset
import android.service.autofill.FillCallback
import android.service.autofill.FillRequest
import android.service.autofill.FillResponse
import android.service.autofill.SaveCallback
import android.service.autofill.SaveRequest
import android.service.autofill.AutofillService
import android.view.autofill.AutofillId
import android.view.autofill.AutofillValue
import android.widget.RemoteViews
import com.lados.desktop.MainActivity
import com.lados.desktop.R

/**
 * System autofill provider for apps and browsers.
 * Needs an unlocked vault session (cache filled from the Lados UI).
 */
class LadosAutofillService : AutofillService() {
  override fun onFillRequest(
    request: FillRequest,
    cancellationSignal: CancellationSignal,
    callback: FillCallback,
  ) {
    val structure: AssistStructure = request.fillContexts.lastOrNull()?.structure
      ?: run {
        callback.onSuccess(null)
        return
      }

    val parsed = AssistStructureParser.parse(structure)
    if (parsed.usernameIds.isEmpty() && parsed.passwordIds.isEmpty()) {
      callback.onSuccess(null)
      return
    }

    val fieldIds = (parsed.usernameIds + parsed.passwordIds).toTypedArray()

    if (!AutofillVaultCache.unlocked) {
      callback.onSuccess(unlockResponse(fieldIds))
      return
    }

    val matches = AutofillVaultCache.matchesForHost(parsed.webDomain)
    if (matches.isEmpty()) {
      callback.onSuccess(unlockResponse(fieldIds))
      return
    }

    val response = FillResponse.Builder()
    for (entry in matches.take(5)) {
      response.addDataset(datasetFor(entry, parsed, presentationFor(entry)))
    }
    callback.onSuccess(response.build())
  }

  override fun onSaveRequest(request: SaveRequest, callback: SaveCallback) {
    callback.onSuccess()
  }

  private fun datasetFor(
    entry: AutofillVaultCache.Entry,
    parsed: ParsedStructure,
    presentation: RemoteViews,
  ): Dataset {
    val dataset = Dataset.Builder(presentation)
    for (id in parsed.usernameIds) {
      dataset.setValue(id, AutofillValue.forText(entry.username))
    }
    for (id in parsed.passwordIds) {
      dataset.setValue(id, AutofillValue.forText(entry.password))
    }
    return dataset.build()
  }

  private fun presentationFor(entry: AutofillVaultCache.Entry): RemoteViews {
    return RemoteViews(packageName, android.R.layout.simple_list_item_1).apply {
      setTextViewText(
        android.R.id.text1,
        buildString {
          append(entry.title.ifBlank { "Lados" })
          if (entry.username.isNotBlank()) {
            append(" — ")
            append(entry.username)
          }
        },
      )
    }
  }

  private fun unlockResponse(fieldIds: Array<AutofillId>): FillResponse {
    val authIntent = Intent(this, MainActivity::class.java).apply {
      action = Intent.ACTION_MAIN
      addCategory(Intent.CATEGORY_LAUNCHER)
      flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP
      putExtra(EXTRA_AUTOFILL_UNLOCK, true)
    }
    val pending = PendingIntent.getActivity(
      this,
      4401,
      authIntent,
      PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
    )
    val presentation = RemoteViews(packageName, android.R.layout.simple_list_item_1).apply {
      setTextViewText(android.R.id.text1, getString(R.string.autofill_unlock_lados))
    }
    return FillResponse.Builder()
      .setAuthentication(fieldIds, pending.intentSender, presentation)
      .build()
  }

  companion object {
    const val EXTRA_AUTOFILL_UNLOCK = "com.lados.desktop.AUTOFILL_UNLOCK"
  }
}
