import { abilityDelegatorRegistry, TestRunner } from '@kit.TestKit';
import { Hypium } from '@ohos/hypium';
import counterTest from '../test/Counter.test';

/** Run the application's native UI tests with HarmonyOS instrumentation. */
export default class OpenHarmonyTestRunner implements TestRunner {
  onPrepare(): void {}

  onRun(): void {
    Hypium.hypiumTest(
      abilityDelegatorRegistry.getAbilityDelegator(),
      abilityDelegatorRegistry.getArguments(),
      counterTest
    );
  }
}
