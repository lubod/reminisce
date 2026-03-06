package org.openreminisce.app

import com.journeyapps.barcodescanner.CaptureActivity

/**
 * Custom CaptureActivity to force the ZXing barcode scanner into portrait orientation.
 * By default, ZXing opens in landscape mode, causing the scanning laser to appear
 * as if it is scanning left-to-right when the phone is held in portrait.
 */
class CustomScannerActivity : CaptureActivity()
