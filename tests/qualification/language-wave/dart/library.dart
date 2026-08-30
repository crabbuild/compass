library wave;

import 'dart:async';
import 'package:flutter/widgets.dart' as widgets;
export 'src/model.dart' show User;
part 'src/part.dart';

abstract class Store {
  Future<void> save(String value);
}

class UserStore implements Store {
  UserStore();
  UserStore.named(this.value);
  final String value;
  @override
  Future<void> save(String value) async {}
  void route(widgets.BuildContext context) {
    widgets.Navigator.of(context).pushNamed('/users');
  }
}

void dynamicCall(dynamic receiver) {
  receiver.unknown();
}
