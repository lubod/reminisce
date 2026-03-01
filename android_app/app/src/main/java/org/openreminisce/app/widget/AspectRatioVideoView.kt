package org.openreminisce.app.widget

import android.content.Context
import android.util.AttributeSet
import android.widget.VideoView
import kotlin.math.roundToInt

class AspectRatioVideoView : VideoView {
    private var videoAspectRatio = 0f

    constructor(context: Context) : super(context)
    constructor(context: Context, attrs: AttributeSet?) : super(context, attrs)
    constructor(context: Context, attrs: AttributeSet?, defStyleAttr: Int) : super(context, attrs, defStyleAttr)

    fun setAspectRatio(width: Int, height: Int) {
        videoAspectRatio = if (height == 0) 0f else width.toFloat() / height.toFloat()
        requestLayout()
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        super.onMeasure(widthMeasureSpec, heightMeasureSpec)
        val width = measuredWidth
        val height = if (videoAspectRatio == 0f) measuredHeight else (width / videoAspectRatio).roundToInt()
        setMeasuredDimension(width, height)
    }
}