import 'package:flutter/material.dart';

/// The virtualised feed `autoscroll` and `busy` scroll: 2,000 cards, each
/// exactly 72 pt high.
class Feed extends StatelessWidget {
  const Feed({super.key, required this.controller});

  static const int itemCount = 2000;
  static const double itemExtent = 72;

  final ScrollController controller;

  @override
  Widget build(BuildContext context) {
    return ListView.builder(
      controller: controller,
      itemExtent: itemExtent,
      itemCount: itemCount,
      itemBuilder: (context, index) => FeedItem(index: index),
    );
  }
}

class FeedItem extends StatelessWidget {
  const FeedItem({super.key, required this.index});

  final int index;

  @override
  Widget build(BuildContext context) {
    final body = 'This is the body of item $index.';
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
      child: Row(
        children: [
          SizedBox.square(
            dimension: 40,
            child: ColoredBox(
              color: HSLColor.fromAHSL(
                1,
                (index * 37 % 360).toDouble(),
                0.6,
                0.5,
              ).toColor(),
            ),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              mainAxisAlignment: MainAxisAlignment.center,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  'Item $index',
                  maxLines: 1,
                  style: const TextStyle(
                    fontSize: 15,
                    fontWeight: FontWeight.w600,
                  ),
                ),
                Text(
                  '$body $body',
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(fontSize: 12),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
