package dev.leptoskit.arena.lynx

import com.lynx.tasm.behavior.Behavior
import com.lynx.tasm.behavior.LynxContext
import com.lynx.tasm.behavior.shadow.ShadowNode
import com.lynx.tasm.behavior.ui.LynxUI
import com.lynx.xelement.LynxUIInputAutoRegistry
import com.lynx.xelement.LynxUIInputShadowNodeAutoRegistry

/**
 * `<input>` from Lynx's XElement library, used by the size-tier form screens.
 * It is registered on its own: `XElementBehaviors` registers every XElement
 * (video, web view, Markdown, SVG, …) and its Maven artifact depends on all
 * of their modules, which would bloat the APK the size tiers measure.
 */
class InputBehavior : Behavior("input", false, false, false) {
	override fun createUI(context: LynxContext): LynxUI<*> = LynxUIInputAutoRegistry(context)

	override fun createShadowNode(): ShadowNode = LynxUIInputShadowNodeAutoRegistry()
}
