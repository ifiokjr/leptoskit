import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';

/// Every element below [root] whose widget is a [T], in tree order.
List<Element> descendantsOfType<T extends Widget>(Element root) {
  final found = <Element>[];
  void visit(Element element) {
    if (element.widget is T) found.add(element);
    element.visitChildren(visit);
  }

  root.visitChildren(visit);
  return found;
}

/// The text a [Text] element has laid out, read from its render object, which
/// is what the frame actually painted.
String shownText(Element textElement) {
  final paragraph = textElement.findRenderObject();
  if (paragraph is! RenderParagraph) {
    throw StateError('${textElement.widget} has no RenderParagraph');
  }
  return paragraph.text.toPlainText();
}

/// The texts painted below [root], in tree order.
List<String> shownTexts(Element root) =>
    descendantsOfType<Text>(root).map(shownText).toList();
