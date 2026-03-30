import { Component, OnInit, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Router } from '@angular/router';
import { AuthService } from '../../core/services/auth.service';

@Component({
  selector: 'app-login',
  standalone: true,
  imports: [CommonModule, FormsModule],
  template: `
    <div class="login-wrapper">
      <div class="login-card">
        <div class="login-header">
          <span class="login-logo">rustnzb</span>
          @if (isSetup()) {
            <p class="login-subtitle">Create your account to get started</p>
          } @else {
            <p class="login-subtitle">Sign in to continue</p>
          }
        </div>

        @if (loading()) {
          <div class="login-loading">Checking status...</div>
        } @else {
          <form (ngSubmit)="onSubmit()" class="login-form">
            @if (errorMessage()) {
              <div class="login-error">{{ errorMessage() }}</div>
            }

            <div class="form-group">
              <label class="form-label" for="username">Username</label>
              <input
                id="username"
                type="text"
                class="form-input"
                [(ngModel)]="username"
                name="username"
                autocomplete="username"
                required
              />
            </div>

            <div class="form-group">
              <label class="form-label" for="password">Password</label>
              <input
                id="password"
                type="password"
                class="form-input"
                [(ngModel)]="password"
                name="password"
                autocomplete="current-password"
                required
              />
            </div>

            @if (isSetup()) {
              <div class="form-group">
                <label class="form-label" for="confirmPassword">Confirm Password</label>
                <input
                  id="confirmPassword"
                  type="password"
                  class="form-input"
                  [(ngModel)]="confirmPassword"
                  name="confirmPassword"
                  autocomplete="new-password"
                  required
                />
              </div>
            }

            <button type="submit" class="submit-btn" [disabled]="submitting()">
              @if (submitting()) {
                @if (isSetup()) { Creating Account... } @else { Signing In... }
              } @else {
                @if (isSetup()) { Create Account } @else { Sign In }
              }
            </button>
          </form>
        }
      </div>
    </div>
  `,
  styles: [`
    .login-wrapper {
      display: flex; align-items: center; justify-content: center;
      min-height: 100vh; background: #0d1117; padding: 16px;
    }

    .login-card {
      width: 100%; max-width: 400px; background: #161b22;
      border: 1px solid #30363d; border-radius: 8px; padding: 32px;
    }

    .login-header { text-align: center; margin-bottom: 24px; }

    .login-logo {
      font-size: 28px; font-weight: 700; color: #58a6ff;
      letter-spacing: -0.5px;
    }

    .login-subtitle {
      color: #8b949e; font-size: 14px; margin: 8px 0 0;
    }

    .login-loading {
      text-align: center; color: #8b949e; padding: 24px 0;
    }

    .login-form { display: flex; flex-direction: column; gap: 16px; }

    .login-error {
      background: rgba(248, 81, 73, 0.1); border: 1px solid #f85149;
      border-radius: 6px; padding: 10px 14px; color: #f85149;
      font-size: 13px;
    }

    .form-group { display: flex; flex-direction: column; gap: 6px; }

    .form-label { color: #c9d1d9; font-size: 13px; font-weight: 600; }

    .form-input {
      background: #0d1117; border: 1px solid #30363d; border-radius: 6px;
      padding: 8px 12px; color: #c9d1d9; font-size: 14px;
      outline: none; transition: border-color 0.15s ease;
    }
    .form-input:focus { border-color: #58a6ff; }
    .form-input::placeholder { color: #484f58; }

    .submit-btn {
      background: #238636; border: 1px solid #2ea043; border-radius: 6px;
      padding: 10px 16px; color: #ffffff; font-size: 14px; font-weight: 600;
      cursor: pointer; transition: background 0.15s ease; margin-top: 4px;
    }
    .submit-btn:hover:not(:disabled) { background: #2ea043; }
    .submit-btn:disabled { opacity: 0.6; cursor: not-allowed; }
  `],
})
export class LoginComponent implements OnInit {
  username = '';
  password = '';
  confirmPassword = '';

  isSetup = signal(false);
  loading = signal(true);
  submitting = signal(false);
  errorMessage = signal('');

  constructor(
    private authService: AuthService,
    private router: Router,
  ) {}

  ngOnInit(): void {
    // If already logged in, go to queue
    if (this.authService.isLoggedIn()) {
      this.router.navigate(['/queue']);
      return;
    }

    this.authService.checkAuth().subscribe({
      next: (status) => {
        if (!status.auth_enabled && !status.setup_required) {
          // Auth is disabled, go straight to queue
          this.router.navigate(['/queue']);
          return;
        }
        this.isSetup.set(status.setup_required);
        this.loading.set(false);
      },
      error: () => {
        // If we can't reach the server, show login anyway
        this.loading.set(false);
      },
    });
  }

  onSubmit(): void {
    this.errorMessage.set('');

    if (!this.username.trim() || !this.password.trim()) {
      this.errorMessage.set('Username and password are required.');
      return;
    }

    if (this.isSetup() && this.password !== this.confirmPassword) {
      this.errorMessage.set('Passwords do not match.');
      return;
    }

    this.submitting.set(true);

    const request$ = this.isSetup()
      ? this.authService.setup(this.username, this.password)
      : this.authService.login(this.username, this.password);

    request$.subscribe({
      next: () => {
        this.router.navigate(['/queue']);
      },
      error: (err) => {
        this.submitting.set(false);
        if (err.status === 401) {
          this.errorMessage.set('Invalid username or password.');
        } else if (err.status === 409) {
          this.errorMessage.set('An account already exists. Please sign in instead.');
        } else if (err.error?.message) {
          this.errorMessage.set(err.error.message);
        } else {
          this.errorMessage.set('An error occurred. Please try again.');
        }
      },
    });
  }
}
