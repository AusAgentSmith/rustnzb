import { Component, OnInit, OnDestroy, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { HttpClient } from '@angular/common/http';
import { MatIconModule } from '@angular/material/icon';
import { MatButtonModule } from '@angular/material/button';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatSnackBar, MatSnackBarModule } from '@angular/material/snack-bar';
import { ApiService } from '../../core/services/api.service';
import { NzbJob, QueueResponse } from '../../core/models/queue.model';

interface CategoryConfig {
  name: string;
  output_dir: string | null;
  post_processing: number;
}

@Component({
  selector: 'app-queue-view',
  standalone: true,
  imports: [CommonModule, FormsModule, MatIconModule, MatButtonModule, MatProgressBarModule, MatSnackBarModule],
  template: `
    <div class="queue-toolbar">
      <span class="queue-stats">{{ jobs().length }} items · {{ formatBytes(remainingBytes()) }} remaining</span>
      <button class="toolbar-btn" (click)="showAddPanel = !showAddPanel">
        @if (showAddPanel) { Hide Add NZB } @else { Add NZB }
      </button>
    </div>

    @if (showAddPanel) {
      <div class="add-panel">
        <div class="add-tabs">
          <button class="tab-btn" [class.active]="addMode === 'file'" (click)="addMode = 'file'">Upload File</button>
          <button class="tab-btn" [class.active]="addMode === 'url'" (click)="addMode = 'url'">From URL</button>
        </div>

        @if (addMode === 'file') {
          <div class="add-form">
            <div class="form-row">
              <input type="file" accept=".nzb" class="file-input" (change)="onFileSelected($event)" />
            </div>
            <div class="form-row">
              <label class="form-label">Category</label>
              <select class="form-select" [(ngModel)]="addCategory">
                <option value="">None</option>
                @for (cat of categories(); track cat.name) {
                  <option [value]="cat.name">{{ cat.name }}</option>
                }
              </select>
              <label class="form-label">Priority</label>
              <select class="form-select" [(ngModel)]="addPriority">
                <option [ngValue]="0">Low</option>
                <option [ngValue]="1">Normal</option>
                <option [ngValue]="2">High</option>
                <option [ngValue]="3">Force</option>
              </select>
            </div>
            <div class="form-row">
              <button class="submit-btn" [disabled]="!selectedFile || uploading" (click)="uploadFile()">
                @if (uploading) { Uploading... } @else { Upload }
              </button>
            </div>
          </div>
        }

        @if (addMode === 'url') {
          <div class="add-form">
            <div class="form-row">
              <input type="text" class="form-input" placeholder="https://example.com/file.nzb"
                     [(ngModel)]="addUrl" />
            </div>
            <div class="form-row">
              <label class="form-label">Category</label>
              <select class="form-select" [(ngModel)]="addCategory">
                <option value="">None</option>
                @for (cat of categories(); track cat.name) {
                  <option [value]="cat.name">{{ cat.name }}</option>
                }
              </select>
              <label class="form-label">Priority</label>
              <select class="form-select" [(ngModel)]="addPriority">
                <option [ngValue]="0">Low</option>
                <option [ngValue]="1">Normal</option>
                <option [ngValue]="2">High</option>
                <option [ngValue]="3">Force</option>
              </select>
            </div>
            <div class="form-row">
              <button class="submit-btn" [disabled]="!addUrl || uploading" (click)="addFromUrl()">
                @if (uploading) { Adding... } @else { Add }
              </button>
            </div>
          </div>
        }
      </div>
    }

    <div class="job-list">
      @for (job of jobs(); track job.id) {
        <div class="job-item">
          <div class="job-icon">{{ statusIcon(job.status) }}</div>
          <div class="job-info">
            <div class="job-name">{{ job.name }}</div>
            <div class="job-meta">
              <span class="tag" [class]="'tag-' + job.status">{{ job.status | uppercase }}</span>
              <span>{{ formatBytes(job.total_bytes) }}</span>
              <select class="inline-select" [ngModel]="job.priority" (ngModelChange)="changeJobPriority(job.id, $event)">
                <option [ngValue]="0">Low</option>
                <option [ngValue]="1">Normal</option>
                <option [ngValue]="2">High</option>
                <option [ngValue]="3">Force</option>
              </select>
              <select class="inline-select" [ngModel]="job.category" (ngModelChange)="changeJobCategory(job.id, $event)">
                <option value="">None</option>
                @for (cat of categories(); track cat.name) {
                  <option [value]="cat.name">{{ cat.name }}</option>
                }
              </select>
              @if (job.status === 'downloading' && job.speed_bps > 0) {
                <span>ETA {{ eta(job) }}</span>
              }
            </div>
          </div>
          @if (job.status === 'downloading' && job.speed_bps > 0) {
            <div class="job-speed">{{ formatSpeed(job.speed_bps) }}</div>
          }
          <div class="job-progress">
            <div class="progress-bar">
              <div class="progress-fill" [class]="progressClass(job.status)"
                   [style.width.%]="percent(job)"></div>
            </div>
            <div class="progress-text">
              @if (job.status === 'downloading') {
                {{ percent(job) }}% · {{ formatBytes(job.downloaded_bytes) }} / {{ formatBytes(job.total_bytes) }}
              } @else if (job.status === 'queued') {
                Waiting...
              } @else if (job.status === 'paused') {
                {{ percent(job) }}% · Paused
              } @else {
                {{ job.status }}
              }
            </div>
          </div>
          <div class="job-actions">
            @if (job.status === 'downloading' || job.status === 'queued') {
              <button class="action-btn" (click)="pauseJob(job.id)" title="Pause">⏸</button>
            }
            @if (job.status === 'paused') {
              <button class="action-btn" (click)="resumeJob(job.id)" title="Resume">▶</button>
            }
            <button class="action-btn" (click)="deleteJob(job.id)" title="Remove">✕</button>
          </div>
        </div>
      }

      @if (jobs().length === 0) {
        <div class="empty-state">
          <div class="empty-icon">📥</div>
          <p>No downloads in queue</p>
          <p class="hint">Add an NZB file or browse newsgroups to get started</p>
        </div>
      }
    </div>
  `,
  styles: [`
    :host { display: flex; flex-direction: column; height: 100%; }
    .queue-toolbar {
      display: flex; align-items: center; justify-content: space-between; padding: 10px 16px;
      background: #0d1117; border-bottom: 1px solid #21262d; font-size: 12px; color: #8b949e;
    }
    .toolbar-btn {
      padding: 4px 12px; border-radius: 4px; border: 1px solid #30363d;
      background: #21262d; color: #c9d1d9; cursor: pointer; font-size: 12px;
    }
    .toolbar-btn:hover { background: #30363d; }
    .add-panel {
      background: #161b22; border-bottom: 1px solid #21262d; padding: 12px 16px;
    }
    .add-tabs {
      display: flex; gap: 4px; margin-bottom: 12px;
    }
    .tab-btn {
      padding: 4px 12px; border-radius: 4px; border: 1px solid #30363d;
      background: #0d1117; color: #8b949e; cursor: pointer; font-size: 12px;
    }
    .tab-btn.active { background: #21262d; color: #c9d1d9; border-color: #58a6ff; }
    .tab-btn:hover { color: #c9d1d9; }
    .add-form { display: flex; flex-direction: column; gap: 8px; }
    .form-row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
    .form-label { font-size: 12px; color: #8b949e; }
    .form-input {
      flex: 1; min-width: 200px; padding: 4px 8px; border-radius: 4px; border: 1px solid #30363d;
      background: #0d1117; color: #c9d1d9; font-size: 12px; outline: none;
    }
    .form-input:focus { border-color: #58a6ff; }
    .form-select {
      padding: 4px 8px; border-radius: 4px; border: 1px solid #30363d;
      background: #0d1117; color: #c9d1d9; font-size: 12px; outline: none;
    }
    .form-select:focus { border-color: #58a6ff; }
    .file-input {
      font-size: 12px; color: #c9d1d9;
    }
    .file-input::file-selector-button {
      padding: 4px 12px; border-radius: 4px; border: 1px solid #30363d;
      background: #21262d; color: #c9d1d9; cursor: pointer; font-size: 12px; margin-right: 8px;
    }
    .file-input::file-selector-button:hover { background: #30363d; }
    .submit-btn {
      padding: 4px 16px; border-radius: 4px; border: 1px solid #238636;
      background: #238636; color: #fff; cursor: pointer; font-size: 12px; font-weight: 600;
    }
    .submit-btn:hover { background: #2ea043; }
    .submit-btn:disabled { opacity: 0.5; cursor: not-allowed; }
    .inline-select {
      padding: 1px 4px; border-radius: 4px; border: 1px solid #30363d;
      background: #0d1117; color: #c9d1d9; font-size: 11px; outline: none; cursor: pointer;
    }
    .inline-select:focus { border-color: #58a6ff; }
    .job-list { flex: 1; overflow-y: auto; }
    .job-item {
      display: flex; align-items: center; gap: 12px;
      padding: 12px 16px; border-bottom: 1px solid #21262d;
    }
    .job-item:hover { background: #161b22; }
    .job-icon { font-size: 20px; width: 24px; text-align: center; }
    .job-info { flex: 1; min-width: 0; }
    .job-name { font-weight: 600; font-size: 14px; margin-bottom: 4px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .job-meta { display: flex; gap: 12px; font-size: 11px; color: #8b949e; flex-wrap: wrap; align-items: center; }
    .tag { padding: 1px 6px; border-radius: 3px; font-size: 10px; font-weight: 600; }
    .tag-downloading { background: #0d419d; color: #58a6ff; }
    .tag-queued { background: #1c2128; color: #8b949e; }
    .tag-paused { background: #3d1d00; color: #d29922; }
    .tag-verifying, .tag-repairing, .tag-extracting, .tag-post_processing { background: #1a3a1a; color: #3fb950; }
    .tag-completed { background: #1a3a1a; color: #3fb950; }
    .tag-failed { background: #3d1418; color: #f85149; }
    .job-speed { font-family: monospace; font-size: 12px; color: #58a6ff; width: 80px; text-align: right; }
    .job-progress { width: 200px; }
    .progress-bar { height: 6px; background: #21262d; border-radius: 3px; overflow: hidden; margin-bottom: 4px; }
    .progress-fill { height: 100%; border-radius: 3px; transition: width 0.3s; }
    .progress-fill.blue { background: linear-gradient(90deg, #1f6feb, #58a6ff); }
    .progress-fill.green { background: #3fb950; }
    .progress-fill.yellow { background: #d29922; }
    .progress-text { font-size: 11px; color: #8b949e; text-align: right; }
    .job-actions { display: flex; gap: 4px; }
    .action-btn {
      padding: 4px 8px; border-radius: 4px; border: 1px solid #30363d;
      background: #21262d; color: #c9d1d9; cursor: pointer; font-size: 12px;
    }
    .action-btn:hover { background: #30363d; }
    .empty-state { text-align: center; padding: 64px 16px; color: #484f58; }
    .empty-icon { font-size: 48px; margin-bottom: 16px; }
    .hint { font-size: 12px; margin-top: 8px; }
  `],
})
export class QueueViewComponent implements OnInit, OnDestroy {
  jobs = signal<NzbJob[]>([]);
  remainingBytes = signal(0);
  categories = signal<CategoryConfig[]>([]);
  private pollTimer: ReturnType<typeof setInterval> | null = null;

  // Add NZB panel state
  showAddPanel = false;
  addMode: 'file' | 'url' = 'file';
  selectedFile: File | null = null;
  addUrl = '';
  addCategory = '';
  addPriority = 1;
  uploading = false;

  constructor(private api: ApiService, private http: HttpClient, private snackBar: MatSnackBar) {}

  ngOnInit(): void {
    this.loadQueue();
    this.loadCategories();
    this.pollTimer = setInterval(() => this.loadQueue(), 2000);
  }

  ngOnDestroy(): void {
    if (this.pollTimer) clearInterval(this.pollTimer);
  }

  loadQueue(): void {
    this.api.get<QueueResponse>('/queue').subscribe({
      next: (r) => {
        this.jobs.set(r.jobs);
        this.remainingBytes.set(r.jobs.reduce((sum, j) => sum + (j.total_bytes - j.downloaded_bytes), 0));
      },
      error: () => {},
    });
  }

  loadCategories(): void {
    this.api.get<CategoryConfig[]>('/config/categories').subscribe({
      next: (cats) => this.categories.set(cats),
      error: () => {},
    });
  }

  // ---- Add NZB (file upload) ----

  onFileSelected(event: Event): void {
    const input = event.target as HTMLInputElement;
    this.selectedFile = input.files?.[0] ?? null;
  }

  uploadFile(): void {
    if (!this.selectedFile || this.uploading) return;
    this.uploading = true;

    const formData = new FormData();
    formData.append('file', this.selectedFile, this.selectedFile.name);

    const params: string[] = [];
    if (this.addCategory) params.push(`category=${encodeURIComponent(this.addCategory)}`);
    if (this.addPriority !== 1) params.push(`priority=${this.addPriority}`);
    const qs = params.length > 0 ? '?' + params.join('&') : '';

    this.http.post(`/api/queue/add${qs}`, formData).subscribe({
      next: () => {
        this.snackBar.open('NZB added to queue', 'Close', { duration: 3000 });
        this.selectedFile = null;
        this.uploading = false;
        this.loadQueue();
      },
      error: (err) => {
        this.snackBar.open('Failed to upload: ' + (err.error?.message || err.statusText), 'Close', { duration: 5000 });
        this.uploading = false;
      },
    });
  }

  // ---- Add NZB (from URL) ----

  addFromUrl(): void {
    if (!this.addUrl || this.uploading) return;
    this.uploading = true;

    const body: { url: string; category?: string; priority?: number } = { url: this.addUrl };
    if (this.addCategory) body.category = this.addCategory;
    if (this.addPriority !== 1) body.priority = this.addPriority;

    this.api.post('/queue/add-url', body).subscribe({
      next: () => {
        this.snackBar.open('NZB added from URL', 'Close', { duration: 3000 });
        this.addUrl = '';
        this.uploading = false;
        this.loadQueue();
      },
      error: (err: any) => {
        this.snackBar.open('Failed to add URL: ' + (err.error?.message || err.statusText), 'Close', { duration: 5000 });
        this.uploading = false;
      },
    });
  }

  // ---- Per-job priority & category ----

  changeJobPriority(id: string, priority: number): void {
    this.api.put(`/queue/${id}/priority`, { priority }).subscribe({
      next: () => this.loadQueue(),
      error: () => this.snackBar.open('Failed to update priority', 'Close', { duration: 3000 }),
    });
  }

  changeJobCategory(id: string, category: string): void {
    this.api.put(`/queue/${id}/category`, { category }).subscribe({
      next: () => this.loadQueue(),
      error: () => this.snackBar.open('Failed to update category', 'Close', { duration: 3000 }),
    });
  }

  // ---- Existing job actions ----

  pauseJob(id: string): void {
    this.api.post(`/queue/${id}/pause`).subscribe(() => this.loadQueue());
  }

  resumeJob(id: string): void {
    this.api.post(`/queue/${id}/resume`).subscribe(() => this.loadQueue());
  }

  deleteJob(id: string): void {
    this.api.delete(`/queue/${id}`).subscribe(() => {
      this.loadQueue();
      this.snackBar.open('Removed from queue', 'Close', { duration: 2000 });
    });
  }

  // ---- Formatting helpers ----

  percent(job: NzbJob): number {
    if (job.total_bytes === 0) return 0;
    return Math.round((job.downloaded_bytes / job.total_bytes) * 100);
  }

  eta(job: NzbJob): string {
    if (job.speed_bps === 0) return '∞';
    const remaining = job.total_bytes - job.downloaded_bytes;
    const secs = remaining / job.speed_bps;
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    const s = Math.floor(secs % 60);
    return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}` : `${m}:${String(s).padStart(2, '0')}`;
  }

  statusIcon(status: string): string {
    const icons: Record<string, string> = {
      downloading: '📥', queued: '⏳', paused: '⏸', verifying: '🔍',
      repairing: '🔧', extracting: '📦', completed: '✅', failed: '❌',
    };
    return icons[status] || '⏳';
  }

  priorityLabel(p: number): string {
    return ['Low', 'Normal', 'High', 'Force'][p] || 'Normal';
  }

  progressClass(status: string): string {
    if (status === 'paused') return 'yellow';
    if (['verifying', 'repairing', 'extracting', 'completed'].includes(status)) return 'green';
    return 'blue';
  }

  formatSpeed(bps: number): string {
    if (bps === 0) return '';
    const k = 1024;
    const sizes = ['B/s', 'KB/s', 'MB/s', 'GB/s'];
    const i = Math.floor(Math.log(bps) / Math.log(k));
    return parseFloat((bps / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
  }

  formatBytes(bytes: number): string {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
  }
}
