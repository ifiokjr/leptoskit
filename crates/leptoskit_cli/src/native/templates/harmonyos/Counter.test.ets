import { abilityDelegatorRegistry, Component, Driver, ON } from '@kit.TestKit';
import { describe, expect, it } from '@ohos/hypium';

/** Wait for a visible native control; a missing render fails within ten seconds. */
async function counterValue(driver: Driver, value: number): Promise<void> {
  const text = `Count: ${value}`;
  const label: Component | undefined = await driver.waitForComponent(ON.text(text).type('Text'), 10000);

  if (label === undefined) {
    throw new Error(`Native counter did not display ${text}`);
  }
  expect(await label.getText()).assertEqual(text);
}

/** Exercise the real ArkUI → Node-API → Rust/Wasm → ArkUI round trip. */
export default function counterTest(): void {
  describe('HarmonyCounter', () => {
    it('incrementsThroughNativeWidgets', 0, async () => {
      const driver = Driver.create();
      const delegator = abilityDelegatorRegistry.getAbilityDelegator();
      await delegator.startAbility({ bundleName: '__BUNDLE_NAME__', abilityName: 'EntryAbility' });
      await counterValue(driver, 0);

      for (let count = 1; count <= 3; count++) {
        const button: Component | undefined = await driver.waitForComponent(ON.text('Increment').type('Button'), 10000);

        if (button === undefined) {
          throw new Error('Native Increment button was not found');
        }
        await button.click();
        await counterValue(driver, count);
      }
    });
  });
}
