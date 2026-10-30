import { UIAbility } from '@kit.AbilityKit';
import { window } from '@kit.ArkUI';

export default class EntryAbility extends UIAbility {
  onForeground(): void { AppStorage.setOrCreate('leptoskit.rive.active', true); }
  onBackground(): void { AppStorage.setOrCreate('leptoskit.rive.active', false); }
  onWindowStageCreate(stage: window.WindowStage): void {
    stage.loadContent('pages/Index');
  }
}
