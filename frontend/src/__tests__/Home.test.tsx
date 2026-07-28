import React from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { Home } from '../components/Home';
import * as apiModule from '../api';

// Mock react-router-dom's Link to avoid router context requirements
vi.mock('react-router-dom', () => ({
  Link: ({ to, children }: { to: string; children: React.ReactNode }) =>
    React.createElement('a', { href: to }, children),
}));

// Mock the api module
vi.mock('../api', () => ({
  api: {
    getAllWorkloads: vi.fn(),
    updateWorkload: vi.fn(),
    upgradeWorkload: vi.fn(),
    refreshAll: vi.fn(),
    getSettings: vi.fn(),
    getNextScheduleTime: vi.fn(),
  },
}));

// Stub out console.error to keep test output clean
const originalConsoleError = console.error;

describe('Home component', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    console.error = originalConsoleError;
  });

  it('shows loading state initially', () => {
    // Don't resolve the promise — component should show loading
    (apiModule.api.getAllWorkloads as ReturnType<typeof vi.fn>).mockReturnValue(
      new Promise(() => {})
    );
    render(<Home />);
    expect(screen.getByText('Loading workloads...')).toBeInTheDocument();
  });

  it('renders workloads after successful fetch', async () => {
    (apiModule.api.getAllWorkloads as ReturnType<typeof vi.fn>).mockResolvedValue([
      {
        name: 'test-app',
        namespace: 'default',
        image: 'nginx',
        current_version: '1.0.0',
        latest_version: '1.0.0',
        last_scanned: '2026-07-28',
        update_available: 'NotAvailable',
        scan_exhausted: 'false',
      },
    ]);

    render(<Home />);

    await waitFor(() => {
      expect(screen.getByText('test-app')).toBeInTheDocument();
    });

    expect(screen.queryByText('Loading workloads...')).not.toBeInTheDocument();
  });

  it('renders empty state when no workloads are found', async () => {
    (apiModule.api.getAllWorkloads as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    render(<Home />);

    await waitFor(() => {
      expect(screen.getByText('No workloads found')).toBeInTheDocument();
    });

    expect(screen.getByText('Click to Refresh All')).toBeInTheDocument();
  });

  it('renders error state when fetch fails', async () => {
    (apiModule.api.getAllWorkloads as ReturnType<typeof vi.fn>).mockRejectedValue(
      new Error('network error')
    );

    // Suppress the console.error from the component's catch block
    console.error = vi.fn();

    render(<Home />);

    await waitFor(() => {
      expect(screen.getByText('Error: Failed to fetch workloads')).toBeInTheDocument();
    });
  });
});
