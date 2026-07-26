import React, { useState, useEffect } from 'react';
import { api } from '../api';
import { Link } from 'react-router-dom';

export const RefreshAll: React.FC = () => {
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [_isComplete, setIsComplete] = useState(false);

  useEffect(() => {
    const refreshAllWorkloads = async () => {
      try {
        await api.refreshAll();
        setIsComplete(true);
      } catch (err) {
        setError('Failed to refresh workloads');
        console.error(err);
      } finally {
        setIsLoading(false);
      }
    };

    refreshAllWorkloads();
  }, []);

  if (isLoading) return <div>Refreshing all workloads...</div>;

  if (error) return <div>Error: {error}</div>;

  return (
    <div>
      <div>Refreshed</div>
      <br />
      <br />
      <Link to="/">Go back to Home</Link>
    </div>
  );
};
