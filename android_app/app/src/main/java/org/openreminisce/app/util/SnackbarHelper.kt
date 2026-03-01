package org.openreminisce.app.util

import android.view.View
import com.google.android.material.snackbar.Snackbar

object SnackbarHelper {

    /**
     * Shows a standard short Snackbar notification
     */
    fun showShortSnackbar(view: View, message: String) {
        Snackbar.make(view, message, Snackbar.LENGTH_SHORT).show()
    }

    /**
     * Shows a standard long Snackbar notification
     */
    fun showLongSnackbar(view: View, message: String) {
        Snackbar.make(view, message, Snackbar.LENGTH_LONG).show()
    }

    /**
     * Shows an indefinite Snackbar that stays until manually dismissed or interacted with
     */
    fun showIndefiniteSnackbar(view: View, message: String) {
        Snackbar.make(view, message, Snackbar.LENGTH_INDEFINITE)
            .setAction(android.R.string.ok) { }
            .show()
    }

    /**
     * Legacy method for compatibility
     */
    fun showSnackbar(view: View, message: String, autoDismiss: Boolean = true) {
        if (autoDismiss) {
            showLongSnackbar(view, message)
        } else {
            showIndefiniteSnackbar(view, message)
        }
    }
}
