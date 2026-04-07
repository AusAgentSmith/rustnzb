import { Component, OnInit, OnDestroy, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { HttpClient } from '@angular/common/http';
import { MatIconModule } from '@angular/material/icon';
import { MatSnackBar, MatSnackBarModule } from '@angular/material/snack-bar';
import { Subscription } from 'rxjs';
import { ApiService } from '../../core/services/api.service';
import { AddNzbService } from '../../core/services/add-nzb.service';
import {
  NzbJob, QueueResponse, HistoryEntry,
  ServerArticleStats, LogEntry, LogsResponse,
} from '../../core/models/queue.model';

interface CategoryConfig {
  name: string;
  output_dir: string | null;
  post_processing: number;
}

@Component({
  selector: 'app-queue-view',
  standalone: true,
  imports: [CommonModule, FormsModule, MatIconModule, MatSnackBarModule],
  template: `
    <!-- Add NZB panel (slides down) -->
    @if (showAddPanel) {
      <div class="add-panel">
        <div class="add-panel-inner">
          <div class="add-tabs">
            <button class="add-tab" [class.active]="addMode === 'file'" (click)="addMode = 'file'">
              <mat-icon>upload_file</mat-icon> Upload Files
            </button>
            <button class="add-tab" [class.active]="addMode === 'url'" (click)="addMode = 'url'">
              <mat-icon>link</mat-icon> From URL
            </button>
            <span class="add-spacer"></span>
            <button class="add-close" (click)="showAddPanel = false">
              <mat-icon>close</mat-icon>
            </button>
          </div>

          @if (addMode === 'file') {
            <div class="dropzone" (dragover)="onDragOver($event)" (dragleave)="onDragLeave($event)" (drop)="onDrop($event)" [class.dragover]="isDragging">
              <mat-icon class="dropzone-icon">cloud_upload</mat-icon>
              <div class="dropzone-text">Drop files here or click to browse</div>
              <div class="dropzone-hint">.nzb, .zip, .rar, .7z, .gz &mdash; multiple files supported</div>
              <input type="file" accept=".nzb,.zip,.rar,.7z,.gz" multiple class="dropzone-input" (change)="onFilesSelected($event)" #fileInput />
            </div>
            @if (selectedFiles.length > 0) {
              <div class="file-chips">
                @for (f of selectedFiles; track f.name) {
                  <div class="file-chip">
                    <span>{{ f.name }}</span>
                    <mat-icon class="chip-remove" (click)="removeFile(f)">close</mat-icon>
                  </div>
                }
              </div>
            }
            <div class="add-options">
              <div class="add-field">
                <label>Category</label>
                <select [(ngModel)]="addCategory">
                  <option value="">None</option>
                  @for (cat of categories(); track cat.name) { <option [value]="cat.name">{{ cat.name }}</option> }
                </select>
              </div>
              <div class="add-field">
                <label>Priority</label>
                <select [(ngModel)]="addPriority">
                  <option [ngValue]="0">Low</option><option [ngValue]="1">Normal</option>
                  <option [ngValue]="2">High</option><option [ngValue]="3">Force</option>
                </select>
              </div>
              <span class="add-spacer"></span>
              <button class="btn btn-primary" [disabled]="selectedFiles.length === 0 || uploading" (click)="uploadFiles()">
                <mat-icon>upload</mat-icon>
                @if (uploading) { Uploading... } @else if (selectedFiles.length > 1) { Upload {{ selectedFiles.length }} files } @else { Upload }
              </button>
            </div>
          }

          @if (addMode === 'url') {
            <div class="url-form">
              <input type="text" class="url-input" placeholder="https://example.com/file.nzb" [(ngModel)]="addUrl" (keydown.enter)="addFromUrl()" />
            </div>
            <div class="add-options">
              <div class="add-field">
                <label>Category</label>
                <select [(ngModel)]="addCategory">
                  <option value="">None</option>
                  @for (cat of categories(); track cat.name) { <option [value]="cat.name">{{ cat.name }}</option> }
                </select>
              </div>
              <div class="add-field">
                <label>Priority</label>
                <select [(ngModel)]="addPriority">
                  <option [ngValue]="0">Low</option><option [ngValue]="1">Normal</option>
                  <option [ngValue]="2">High</option><option [ngValue]="3">Force</option>
                </select>
              </div>
              <span class="add-spacer"></span>
              <button class="btn btn-primary" [disabled]="!addUrl || uploading" (click)="addFromUrl()">
                <mat-icon>add</mat-icon>
                @if (uploading) { Adding... } @else { Add }
              </button>
            </div>
          }
        </div>
      </div>
    }

    <!-- Split layout: list + detail -->
    <div class="split-layout">
      <!-- Left: Job list -->
      <div class="list-panel">
        <!-- Filter bar -->
        <div class="filter-bar">
          <button class="filter-chip" [class.active]="filterStatus === 'all'" (click)="filterStatus = 'all'">All ({{ jobs().length }})</button>
          <button class="filter-chip" [class.active]="filterStatus === 'active'" (click)="filterStatus = 'active'">Active</button>
          <button class="filter-chip" [class.active]="filterStatus === 'queued'" (click)="filterStatus = 'queued'">Queued</button>
          <button class="filter-chip" [class.active]="filterStatus === 'paused'" (click)="filterStatus = 'paused'">Paused</button>
          <span class="filter-spacer"></span>
          <span class="filter-remaining">{{ formatBytes(remainingBytes()) }} remaining</span>
        </div>

        <!-- Job rows -->
        <div class="job-list">
          @for (job of filteredJobs(); track job.id) {
            <div class="job-row" [class.selected]="selectedId() === job.id" (click)="selectJob(job.id)">
              <div class="job-status-icon" [class]="job.status">
                <mat-icon>{{ matStatusIcon(job.status) }}</mat-icon>
              </div>
              <div class="job-info">
                <div class="job-name">{{ job.name }}</div>
                <div class="job-subtitle">
                  @if (priorityLabel(job.priority) !== 'Normal') {
                    <span class="tag tag-priority">{{ priorityLabel(job.priority) }}</span>
                  }
                  @if (job.category) { <span>{{ job.category }}</span> }
                  @if (job.status === 'downloading' && job.speed_bps > 0) {
                    <span>ETA {{ eta(job) }}</span>
                  } @else if (job.status === 'queued') {
                    <span>Waiting...</span>
                  } @else if (job.status === 'paused') {
                    <span>Paused</span>
                  } @else {
                    <span>{{ job.status }}</span>
                  }
                </div>
              </div>
              <div class="job-right">
                <div class="job-percent" [class]="'pct-' + job.status">{{ percent(job) }}%</div>
                <div class="mini-progress">
                  <div class="mini-progress-fill" [class]="progressClass(job.status)" [style.width.%]="percent(job)"></div>
                </div>
              </div>
            </div>
          }

          @if (jobs().length === 0) {
            <div class="empty-state">
              <mat-icon class="empty-icon">cloud_download</mat-icon>
              <div class="empty-text">No downloads in queue</div>
              <div class="empty-hint">Click "Add NZB" to get started</div>
            </div>
          }
        </div>
      </div>

      <!-- Right: Detail panel -->
      <div class="detail-panel" [class.has-selection]="selectedJob() || selectedHistoryEntry()">
        @if (selectedJob(); as job) {
          <!-- Job detail header -->
          <div class="detail-header">
            <div class="detail-title">{{ job.name }}</div>
            <div class="detail-status-row">
              <span class="tag" [class]="'tag-' + job.status">{{ job.status | uppercase }}</span>
              @if (priorityLabel(job.priority) !== 'Normal') {
                <span class="tag tag-priority">{{ priorityLabel(job.priority) }}</span>
              }
              @if (job.speed_bps > 0) {
                <span class="detail-speed">{{ formatSpeed(job.speed_bps) }}</span>
              }
            </div>
          </div>

          <!-- Progress -->
          <div class="detail-progress">
            <div class="detail-progress-bar">
              <div class="detail-progress-fill" [class]="progressClass(job.status)" [style.width.%]="percent(job)"></div>
            </div>
            <div class="detail-progress-labels">
              <span>{{ formatBytes(job.downloaded_bytes) }} / {{ formatBytes(job.total_bytes) }} ({{ percent(job) }}%)</span>
              @if (job.status === 'downloading' && job.speed_bps > 0) {
                <span>ETA {{ eta(job) }}</span>
              }
            </div>
          </div>

          <!-- Tabs -->
          <div class="detail-tabs">
            <button class="dtab" [class.active]="detailTab() === 'info'" (click)="detailTab.set('info')">Info</button>
            <button class="dtab" [class.active]="detailTab() === 'logs'" (click)="detailTab.set('logs'); loadJobLogs(job.id)">Logs</button>
          </div>

          <div class="detail-body">
            @if (detailTab() === 'info') {
              <div class="detail-row"><span class="dr-label">Status</span><span class="dr-value">{{ job.status | uppercase }}</span></div>
              <div class="detail-row"><span class="dr-label">Size</span><span class="dr-value">{{ formatBytes(job.total_bytes) }}</span></div>
              <div class="detail-row"><span class="dr-label">Files</span><span class="dr-value">{{ job.files_completed }} / {{ job.file_count }}</span></div>
              <div class="detail-row"><span class="dr-label">Articles</span><span class="dr-value">{{ job.articles_downloaded }} / {{ job.article_count }}</span></div>
              @if (job.articles_failed > 0) {
                <div class="detail-row"><span class="dr-label">Failed</span><span class="dr-value error-text">{{ job.articles_failed }}</span></div>
              }
              <div class="detail-row"><span class="dr-label">Category</span><span class="dr-value">{{ job.category || 'None' }}</span></div>
              <div class="detail-row"><span class="dr-label">Priority</span><span class="dr-value">{{ priorityLabel(job.priority) }}</span></div>
              <div class="detail-row"><span class="dr-label">Added</span><span class="dr-value">{{ formatDate(job.added_at) }}</span></div>
              @if (job.speed_bps > 0) {
                <div class="detail-row"><span class="dr-label">Speed</span><span class="dr-value speed-text">{{ formatSpeed(job.speed_bps) }}</span></div>
              }
              @if (job.error_message) {
                <div class="detail-row"><span class="dr-label">Error</span><span class="dr-value error-text">{{ job.error_message }}</span></div>
              }

              @if (job.server_stats && job.server_stats.length > 0) {
                <div class="detail-section-title">Server Stats</div>
                @for (ss of job.server_stats; track ss.server_id) {
                  <div class="server-stat">
                    <span class="server-dot" [style.background]="serverColor(ss.server_id)"></span>
                    <span class="server-name">{{ ss.server_name }}</span>
                    <span class="server-count">{{ ss.articles_downloaded }} articles</span>
                  </div>
                }
              }
            }
            @if (detailTab() === 'logs') {
              <div class="log-viewer">
                @if (jobLogs().loading) { <div class="log-empty">Loading logs...</div> }
                @else if (jobLogs().entries.length === 0) { <div class="log-empty">No log entries</div> }
                @else {
                  @for (log of jobLogs().entries; track log.seq) {
                    <div class="log-line" [class]="'log-' + log.level.toLowerCase()">
                      <span class="log-ts">{{ log.timestamp | slice:11:19 }}</span>
                      <span class="log-msg">{{ log.message }}</span>
                    </div>
                  }
                }
              </div>
            }
          </div>

          <!-- Action buttons -->
          <div class="detail-actions">
            @if (job.status === 'downloading' || job.status === 'queued') {
              <button class="btn" (click)="pauseJob(job.id)"><mat-icon>pause</mat-icon> Pause</button>
            }
            @if (job.status === 'paused') {
              <button class="btn" (click)="resumeJob(job.id)"><mat-icon>play_arrow</mat-icon> Resume</button>
            }
            <span class="detail-actions-spacer"></span>
            <button class="btn btn-danger" (click)="deleteJob(job.id)"><mat-icon>delete_outline</mat-icon></button>
          </div>
        }

        @if (!selectedJob() && !selectedHistoryEntry()) {
          <div class="detail-empty">
            <mat-icon class="detail-empty-icon">info_outline</mat-icon>
            <div>Select a job to view details</div>
          </div>
        }
      </div>
    </div>
  `,
  styles: [`
    :host { display: flex; flex-direction: column; height: 100%; overflow: hidden; }

    /* ---- Add NZB Panel ---- */
    .add-panel { background: #161b22; border-bottom: 1px solid #30363d; flex-shrink: 0; }
    .add-panel-inner { padding: 14px 20px; }
    .add-tabs { display: flex; align-items: center; gap: 4px; margin-bottom: 14px; }
    .add-tab {
      display: flex; align-items: center; gap: 6px; padding: 6px 16px; border-radius: 6px;
      border: 1px solid #30363d; background: transparent; color: #8b949e;
      cursor: pointer; font-size: 13px; transition: all 0.15s;
    }
    .add-tab mat-icon { font-size: 18px; width: 18px; height: 18px; }
    .add-tab:hover { color: #c9d1d9; }
    .add-tab.active { background: #21262d; color: #e6edf3; border-color: #484f58; }
    .add-spacer { flex: 1; }
    .add-close {
      background: none; border: none; color: #484f58; cursor: pointer; padding: 4px;
      border-radius: 4px; display: flex; align-items: center;
    }
    .add-close:hover { background: #21262d; color: #c9d1d9; }

    /* Dropzone */
    .dropzone {
      border: 2px dashed #30363d; border-radius: 8px; padding: 28px; text-align: center;
      cursor: pointer; transition: all 0.2s; position: relative;
    }
    .dropzone:hover, .dropzone.dragover { border-color: #f0883e; background: #f0883e08; }
    .dropzone-icon { font-size: 36px !important; width: 36px !important; height: 36px !important; color: #30363d; margin-bottom: 6px; }
    .dropzone:hover .dropzone-icon, .dropzone.dragover .dropzone-icon { color: #f0883e; }
    .dropzone-text { font-size: 14px; color: #8b949e; margin-bottom: 4px; }
    .dropzone-hint { font-size: 12px; color: #484f58; }
    .dropzone-input {
      position: absolute; inset: 0; opacity: 0; cursor: pointer; width: 100%; height: 100%;
    }

    /* File chips */
    .file-chips { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 10px; }
    .file-chip {
      display: flex; align-items: center; gap: 6px; background: #21262d;
      border: 1px solid #30363d; border-radius: 16px; padding: 4px 10px; font-size: 12px; color: #c9d1d9;
    }
    .chip-remove { font-size: 14px !important; width: 14px !important; height: 14px !important; color: #484f58; cursor: pointer; }
    .chip-remove:hover { color: #f85149; }

    /* URL form */
    .url-form { margin-bottom: 10px; }
    .url-input {
      width: 100%; padding: 8px 12px; border-radius: 6px; border: 1px solid #30363d;
      background: #0d1117; color: #c9d1d9; font-size: 13px; outline: none;
    }
    .url-input:focus { border-color: #58a6ff; }
    .url-input::placeholder { color: #484f58; }

    /* Add options row */
    .add-options { display: flex; align-items: flex-end; gap: 14px; margin-top: 12px; }
    .add-field { display: flex; flex-direction: column; gap: 4px; }
    .add-field label { font-size: 11px; color: #8b949e; font-weight: 500; }
    .add-field select {
      background: #0d1117; border: 1px solid #30363d; color: #c9d1d9;
      padding: 6px 10px; border-radius: 6px; font-size: 12px; outline: none;
    }
    .add-field select:focus { border-color: #58a6ff; }

    /* ---- Split Layout ---- */
    .split-layout { flex: 1; display: flex; overflow: hidden; }

    /* Left panel: job list */
    .list-panel { flex: 1; display: flex; flex-direction: column; border-right: 1px solid #21262d; min-width: 0; }

    /* Filter bar */
    .filter-bar {
      display: flex; align-items: center; gap: 6px; padding: 8px 14px;
      border-bottom: 1px solid #21262d; background: #0d1117; flex-shrink: 0;
    }
    .filter-chip {
      padding: 4px 12px; border-radius: 14px; border: 1px solid #30363d;
      background: transparent; color: #8b949e; cursor: pointer; font-size: 12px; transition: all 0.15s;
    }
    .filter-chip:hover { border-color: #484f58; color: #c9d1d9; }
    .filter-chip.active { border-color: #f0883e; color: #f0883e; background: #f0883e11; }
    .filter-spacer { flex: 1; }
    .filter-remaining { font-size: 11px; color: #484f58; font-family: 'JetBrains Mono', Consolas, monospace; }

    /* Job rows */
    .job-list { flex: 1; overflow-y: auto; }
    .job-row {
      display: flex; align-items: center; gap: 12px; padding: 10px 14px;
      border-bottom: 1px solid #161b22; cursor: pointer; transition: all 0.1s;
    }
    .job-row:hover { background: #161b22; }
    .job-row.selected { background: #161b22; border-left: 3px solid #f0883e; padding-left: 11px; }

    .job-status-icon {
      width: 32px; height: 32px; border-radius: 6px;
      display: flex; align-items: center; justify-content: center; flex-shrink: 0;
    }
    .job-status-icon mat-icon { font-size: 18px; width: 18px; height: 18px; }
    .job-status-icon.downloading { background: #0d419d33; color: #58a6ff; }
    .job-status-icon.queued { background: #21262d; color: #8b949e; }
    .job-status-icon.paused { background: #9e6a0333; color: #d29922; }
    .job-status-icon.verifying, .job-status-icon.repairing, .job-status-icon.extracting { background: #23863633; color: #3fb950; }
    .job-status-icon.completed { background: #23863633; color: #3fb950; }
    .job-status-icon.failed { background: #da363433; color: #f85149; }

    .job-info { flex: 1; min-width: 0; }
    .job-name {
      font-size: 13px; font-weight: 600; color: #e6edf3;
      white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
    }
    .job-subtitle { font-size: 11px; color: #484f58; margin-top: 2px; display: flex; align-items: center; gap: 6px; }

    .job-right { text-align: right; flex-shrink: 0; min-width: 80px; }
    .job-percent { font-family: 'JetBrains Mono', Consolas, monospace; font-size: 13px; font-weight: 600; }
    .pct-downloading { color: #58a6ff; }
    .pct-queued { color: #484f58; }
    .pct-paused { color: #d29922; }
    .pct-completed, .pct-verifying, .pct-repairing, .pct-extracting { color: #3fb950; }
    .pct-failed { color: #f85149; }

    .mini-progress { width: 80px; height: 3px; background: #21262d; border-radius: 2px; overflow: hidden; margin-top: 4px; margin-left: auto; }
    .mini-progress-fill { height: 100%; border-radius: 2px; transition: width 0.3s; }
    .mini-progress-fill.blue { background: #58a6ff; }
    .mini-progress-fill.yellow { background: #d29922; }
    .mini-progress-fill.green { background: #3fb950; }

    /* Tags */
    .tag { font-size: 10px; font-weight: 600; padding: 2px 8px; border-radius: 10px; text-transform: uppercase; letter-spacing: 0.3px; }
    .tag-downloading { background: #0d419d33; color: #58a6ff; }
    .tag-queued { background: #30363d; color: #8b949e; }
    .tag-paused { background: #9e6a0333; color: #d29922; }
    .tag-verifying, .tag-repairing, .tag-extracting, .tag-post_processing { background: #23863633; color: #3fb950; }
    .tag-completed, .tag-ok { background: #23863633; color: #3fb950; }
    .tag-failed, .tag-error { background: #da363433; color: #f85149; }
    .tag-priority { background: #f0883e33; color: #f0883e; }

    /* Empty state */
    .empty-state { text-align: center; padding: 48px 16px; color: #484f58; }
    .empty-icon { font-size: 48px !important; width: 48px !important; height: 48px !important; color: #21262d; margin-bottom: 12px; }
    .empty-text { font-size: 14px; font-weight: 600; margin-bottom: 4px; }
    .empty-hint { font-size: 12px; }

    /* ---- Right: Detail panel ---- */
    .detail-panel {
      width: 380px; overflow-y: auto; background: #010409; flex-shrink: 0;
      display: flex; flex-direction: column;
    }

    .detail-header { padding: 18px 20px 14px; border-bottom: 1px solid #21262d; }
    .detail-title { font-size: 15px; font-weight: 700; color: #e6edf3; margin-bottom: 8px; word-break: break-word; }
    .detail-status-row { display: flex; align-items: center; gap: 8px; }
    .detail-speed {
      margin-left: auto; font-family: 'JetBrains Mono', Consolas, monospace;
      font-size: 12px; color: #3fb950;
    }

    .detail-progress { padding: 14px 20px; }
    .detail-progress-bar { height: 8px; background: #21262d; border-radius: 4px; overflow: hidden; }
    .detail-progress-fill { height: 100%; border-radius: 4px; transition: width 0.3s; }
    .detail-progress-fill.blue { background: linear-gradient(90deg, #1f6feb, #58a6ff); }
    .detail-progress-fill.green { background: linear-gradient(90deg, #238636, #3fb950); }
    .detail-progress-fill.yellow { background: linear-gradient(90deg, #9e6a03, #d29922); }
    .detail-progress-labels {
      display: flex; justify-content: space-between; margin-top: 6px;
      font-size: 11px; color: #484f58; font-family: 'JetBrains Mono', Consolas, monospace;
    }

    .detail-tabs { display: flex; border-bottom: 1px solid #21262d; padding: 0 20px; }
    .dtab {
      padding: 8px 14px; border-bottom: 2px solid transparent;
      background: none; border-top: none; border-left: none; border-right: none;
      color: #8b949e; cursor: pointer; font-size: 12px; font-weight: 500; transition: all 0.15s;
    }
    .dtab:hover { color: #c9d1d9; }
    .dtab.active { color: #e6edf3; border-bottom-color: #f0883e; }

    .detail-body { flex: 1; padding: 14px 20px; overflow-y: auto; }
    .detail-row { display: flex; justify-content: space-between; padding: 6px 0; border-bottom: 1px solid #161b2244; }
    .dr-label { font-size: 12px; color: #484f58; }
    .dr-value { font-size: 12px; color: #c9d1d9; font-family: 'JetBrains Mono', Consolas, monospace; }
    .error-text { color: #f85149; }
    .speed-text { color: #3fb950; }

    .detail-section-title {
      margin-top: 16px; margin-bottom: 8px; font-size: 11px; font-weight: 600;
      color: #484f58; text-transform: uppercase; letter-spacing: 0.5px;
    }
    .server-stat { display: flex; align-items: center; gap: 8px; padding: 5px 0; font-size: 12px; }
    .server-dot { width: 8px; height: 8px; border-radius: 50%; flex-shrink: 0; }
    .server-name { color: #c9d1d9; flex: 1; }
    .server-count { color: #8b949e; font-family: 'JetBrains Mono', Consolas, monospace; font-size: 11px; }

    /* Log viewer */
    .log-viewer {
      font-family: 'JetBrains Mono', Consolas, monospace; font-size: 11px;
      background: #0d1117; border: 1px solid #21262d; border-radius: 4px; padding: 8px;
      max-height: 400px; overflow-y: auto;
    }
    .log-empty { color: #484f58; padding: 8px 0; font-size: 12px; }
    .log-line { display: flex; gap: 8px; line-height: 1.6; white-space: pre-wrap; word-break: break-all; }
    .log-ts { color: #30363d; flex-shrink: 0; }
    .log-msg { flex: 1; }
    .log-info .log-msg { color: #8b949e; }
    .log-warn .log-msg { color: #d29922; }
    .log-error .log-msg { color: #f85149; }
    .log-debug .log-msg { color: #6e7681; }

    /* Detail actions */
    .detail-actions {
      display: flex; gap: 8px; padding: 14px 20px; border-top: 1px solid #21262d; flex-shrink: 0;
    }
    .detail-actions-spacer { flex: 1; }

    .detail-empty {
      flex: 1; display: flex; flex-direction: column; align-items: center;
      justify-content: center; color: #30363d; gap: 8px; font-size: 13px;
    }
    .detail-empty-icon { font-size: 40px !important; width: 40px !important; height: 40px !important; }

    /* Buttons */
    .btn {
      padding: 7px 14px; border-radius: 6px; border: 1px solid #30363d;
      background: #21262d; color: #c9d1d9; cursor: pointer; font-size: 12px;
      font-weight: 500; display: flex; align-items: center; gap: 6px; transition: all 0.15s;
    }
    .btn:hover { background: #30363d; }
    .btn mat-icon { font-size: 16px; width: 16px; height: 16px; }
    .btn-primary { background: #238636; border-color: #2ea043; color: white; }
    .btn-primary:hover { background: #2ea043; }
    .btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }
    .btn-danger { background: transparent; border-color: #da3634; color: #f85149; }
    .btn-danger:hover { background: #da363422; }
  `],
})
export class QueueViewComponent implements OnInit, OnDestroy {
  jobs = signal<NzbJob[]>([]);
  history = signal<HistoryEntry[]>([]);
  remainingBytes = signal(0);
  categories = signal<CategoryConfig[]>([]);
  selectedId = signal<string | null>(null);
  detailTab = signal<'info' | 'logs'>('info');
  jobLogs = signal<{ entries: LogEntry[]; loading: boolean }>({ entries: [], loading: false });
  private pollTimer: ReturnType<typeof setInterval> | null = null;

  // Filter
  filterStatus: 'all' | 'active' | 'queued' | 'paused' = 'all';

  // Add NZB panel state
  showAddPanel = false;
  addMode: 'file' | 'url' = 'file';
  selectedFiles: File[] = [];
  addUrl = '';
  addCategory = '';
  addPriority = 1;
  uploading = false;
  isDragging = false;
  private toggleSub: Subscription | null = null;

  constructor(
    private api: ApiService,
    private http: HttpClient,
    private snackBar: MatSnackBar,
    private addNzbService: AddNzbService,
  ) {}

  ngOnInit(): void {
    this.loadQueue();
    this.loadCategories();
    this.pollTimer = setInterval(() => this.loadQueue(), 2000);
    this.toggleSub = this.addNzbService.panelToggle$.subscribe(() => {
      this.showAddPanel = !this.showAddPanel;
    });
  }

  ngOnDestroy(): void {
    if (this.pollTimer) clearInterval(this.pollTimer);
    this.toggleSub?.unsubscribe();
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

  // ---- Filtering ----

  filteredJobs(): NzbJob[] {
    const all = this.jobs();
    if (this.filterStatus === 'all') return all;
    if (this.filterStatus === 'active') return all.filter(j => j.status === 'downloading');
    if (this.filterStatus === 'queued') return all.filter(j => j.status === 'queued');
    if (this.filterStatus === 'paused') return all.filter(j => j.status === 'paused');
    return all;
  }

  // ---- Selection ----

  selectedJob(): NzbJob | null {
    const id = this.selectedId();
    if (!id) return null;
    return this.jobs().find(j => j.id === id) ?? null;
  }

  selectedHistoryEntry(): HistoryEntry | null {
    return null; // History is on its own page now
  }

  selectJob(id: string): void {
    if (this.selectedId() === id) { this.selectedId.set(null); return; }
    this.selectedId.set(id);
    this.detailTab.set('info');
    this.jobLogs.set({ entries: [], loading: false });
  }

  // ---- Logs ----

  loadJobLogs(id: string): void {
    this.jobLogs.set({ entries: [], loading: true });
    this.api.get<LogsResponse>('/logs', { job_id: id }).subscribe({
      next: r => this.jobLogs.set({ entries: r.entries || [], loading: false }),
      error: () => this.jobLogs.set({ entries: [], loading: false }),
    });
  }

  // ---- Add NZB ----

  onDragOver(e: DragEvent): void { e.preventDefault(); this.isDragging = true; }
  onDragLeave(e: DragEvent): void { this.isDragging = false; }
  onDrop(e: DragEvent): void {
    e.preventDefault();
    this.isDragging = false;
    if (e.dataTransfer?.files) {
      this.selectedFiles = [...this.selectedFiles, ...Array.from(e.dataTransfer.files)];
    }
  }

  onFilesSelected(event: Event): void {
    const input = event.target as HTMLInputElement;
    if (input.files) {
      this.selectedFiles = [...this.selectedFiles, ...Array.from(input.files)];
    }
  }

  removeFile(file: File): void {
    this.selectedFiles = this.selectedFiles.filter(f => f !== file);
  }

  uploadFiles(): void {
    if (this.selectedFiles.length === 0 || this.uploading) return;
    this.uploading = true;
    const formData = new FormData();
    for (const file of this.selectedFiles) {
      formData.append('file', file, file.name);
    }
    const params: string[] = [];
    if (this.addCategory) params.push(`category=${encodeURIComponent(this.addCategory)}`);
    if (this.addPriority !== 1) params.push(`priority=${this.addPriority}`);
    const qs = params.length > 0 ? '?' + params.join('&') : '';
    const token = localStorage.getItem('access_token');
    const headers: Record<string, string> = token ? { Authorization: `Bearer ${token}` } : {};
    this.http.post(`/api/queue/add${qs}`, formData, { headers }).subscribe({
      next: () => {
        const count = this.selectedFiles.length;
        this.snackBar.open(`${count} NZB${count > 1 ? 's' : ''} added to queue`, 'Close', { duration: 3000 });
        this.selectedFiles = []; this.uploading = false; this.showAddPanel = false; this.loadQueue();
      },
      error: (err) => {
        const msg = err.error?.message || (err.status === 413 ? 'Upload too large' : err.statusText) || 'Upload failed';
        this.snackBar.open('Failed: ' + msg, 'Close', { duration: 5000 });
        this.uploading = false;
      },
    });
  }

  addFromUrl(): void {
    if (!this.addUrl || this.uploading) return;
    this.uploading = true;
    const body: { url: string; category?: string; priority?: number } = { url: this.addUrl };
    if (this.addCategory) body.category = this.addCategory;
    if (this.addPriority !== 1) body.priority = this.addPriority;
    this.api.post('/queue/add-url', body).subscribe({
      next: () => {
        this.snackBar.open('NZB added from URL', 'Close', { duration: 3000 });
        this.addUrl = ''; this.uploading = false; this.showAddPanel = false; this.loadQueue();
      },
      error: (err: any) => {
        const msg = err.error?.message || (err.status === 413 ? 'Upload too large' : err.statusText) || 'Upload failed';
        this.snackBar.open('Failed: ' + msg, 'Close', { duration: 5000 });
        this.uploading = false;
      },
    });
  }

  // ---- Per-job actions ----

  pauseJob(id: string): void { this.api.post(`/queue/${id}/pause`).subscribe(() => this.loadQueue()); }
  resumeJob(id: string): void { this.api.post(`/queue/${id}/resume`).subscribe(() => this.loadQueue()); }

  deleteJob(id: string): void {
    this.api.delete(`/queue/${id}`).subscribe(() => {
      if (this.selectedId() === id) this.selectedId.set(null);
      this.loadQueue();
    });
  }

  // ---- Formatting ----

  percent(job: { total_bytes: number; downloaded_bytes: number }): number {
    if (job.total_bytes === 0) return 0;
    return Math.round((job.downloaded_bytes / job.total_bytes) * 100);
  }

  eta(job: NzbJob): string {
    if (job.speed_bps === 0) return '--';
    const remaining = job.total_bytes - job.downloaded_bytes;
    const secs = remaining / job.speed_bps;
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    const s = Math.floor(secs % 60);
    return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}` : `${m}:${String(s).padStart(2, '0')}`;
  }

  formatDate(d: string): string {
    if (!d) return '--';
    return new Date(d).toLocaleString();
  }

  matStatusIcon(status: string): string {
    const icons: Record<string, string> = {
      downloading: 'south', queued: 'schedule', paused: 'pause',
      verifying: 'verified', repairing: 'build', extracting: 'unarchive',
      completed: 'check', failed: 'error_outline',
    };
    return icons[status] || 'schedule';
  }

  priorityLabel(p: number): string {
    return ['Low', 'Normal', 'High', 'Force'][p] || 'Normal';
  }

  progressClass(status: string): string {
    if (status === 'paused') return 'yellow';
    if (['verifying', 'repairing', 'extracting', 'completed'].includes(status)) return 'green';
    return 'blue';
  }

  serverColor(serverId: string): string {
    const colors = ['#3fb950', '#58a6ff', '#f0883e', '#d29922', '#bc8cff', '#f778ba'];
    let hash = 0;
    for (const c of serverId) hash = ((hash << 5) - hash + c.charCodeAt(0)) | 0;
    return colors[Math.abs(hash) % colors.length];
  }

  formatSpeed(bps: number): string {
    if (bps === 0) return '0 B/s';
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
