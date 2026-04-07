import { Injectable } from '@angular/core';
import { Subject } from 'rxjs';

@Injectable({ providedIn: 'root' })
export class AddNzbService {
  private toggle$ = new Subject<void>();
  panelToggle$ = this.toggle$.asObservable();

  togglePanel(): void {
    this.toggle$.next();
  }
}
