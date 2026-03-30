import { Component, OnInit, OnDestroy, signal, ElementRef, ViewChild } from '@angular/core';
import { CommonModule } from '@angular/common';
import { ApiService } from '../../core/services/api.service';

interface LogEntry { seq: number; level: string; message: string; timestamp: string; }

@Component({
  selector: 'app-logs-view',
  standalone: true,
  imports: [CommonModule],
  template: `
    <div class="log-container" #logContainer>
      @for (entry of entries(); track entry.seq) {
        <div class="log-line">
          <span class="ts">{{ entry.timestamp }}</span>
          <span class="level" [class]="'level-' + entry.level.toLowerCase()">{{ entry.level }}</span>
          <span class="msg">{{ entry.message }}</span>
        </div>
      }
      @if (entries().length === 0) {
        <div class="empty">No log entries</div>
      }
    </div>
  `,
  styles: [`
    :host { display: flex; height: 100%; }
    .log-container {
      flex: 1; overflow-y: auto; padding: 8px 12px;
      font-family: Consolas, 'Roboto Mono', monospace; font-size: 11px; line-height: 1.8;
      background: #0d1117;
    }
    .log-line { display: flex; gap: 8px; }
    .ts { color: #484f58; min-width: 70px; }
    .level { min-width: 50px; font-weight: 600; }
    .level-info { color: #3fb950; }
    .level-warn, .level-warning { color: #d29922; }
    .level-error { color: #f85149; }
    .level-debug { color: #8b949e; }
    .msg { color: #c9d1d9; }
    .empty { padding: 24px; text-align: center; color: #484f58; }
  `],
})
export class LogsViewComponent implements OnInit, OnDestroy {
  entries = signal<LogEntry[]>([]);
  private lastSeq = 0;
  private pollTimer: ReturnType<typeof setInterval> | null = null;
  @ViewChild('logContainer') logContainer!: ElementRef;

  constructor(private api: ApiService) {}

  ngOnInit(): void {
    this.loadLogs();
    this.pollTimer = setInterval(() => this.loadLogs(), 2000);
  }

  ngOnDestroy(): void {
    if (this.pollTimer) clearInterval(this.pollTimer);
  }

  loadLogs(): void {
    const params: Record<string, string> = {};
    if (this.lastSeq > 0) params['after_seq'] = String(this.lastSeq);
    this.api.get<{ entries: LogEntry[] }>('/logs', params).subscribe({
      next: r => {
        if (r.entries?.length) {
          const all = [...this.entries(), ...r.entries].slice(-500);
          this.entries.set(all);
          this.lastSeq = r.entries[r.entries.length - 1].seq;
          setTimeout(() => {
            const el = this.logContainer?.nativeElement;
            if (el) el.scrollTop = el.scrollHeight;
          }, 50);
        }
      },
      error: () => {},
    });
  }
}
